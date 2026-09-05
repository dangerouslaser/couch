//! The panel: framebuffer output and the Slint platform that drives it.
//!
//! This kernel has no DRM/KMS and no X11 or Wayland, so none of Slint's stock
//! backends apply. We supply a Platform and render into /dev/fb0 ourselves.
//!
//! Two details are load-bearing, both learned the hard way:
//!
//! - The panel reads the low byte as red (fb_var_screeninfo reports red=0/8),
//!   so the buffer is ABGR in memory. Declaring that through TargetPixel means
//!   the renderer writes it directly, instead of a swizzle pass over every
//!   pixel of every frame.
//! - Draw into cached RAM and copy only the dirty rectangles to the mapped
//!   framebuffer. Writing the framebuffer directly during rasterisation is what
//!   made an earlier version cost 98ms a frame instead of one.

use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::PhysicalSize;

/// Memory order is B,G,R,A here because the panel takes red in the low byte.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Default)]
pub struct Abgr(pub u32);

impl TargetPixel for Abgr {
    fn blend(&mut self, c: PremultipliedRgbaColor) {
        let a = (255 - c.alpha) as u32;
        let (r, g, b) = (self.0 & 0xff, (self.0 >> 8) & 0xff, (self.0 >> 16) & 0xff);
        self.0 = 0xff00_0000
            | ((c.blue as u32 + (b * a) / 255) << 16)
            | ((c.green as u32 + (g * a) / 255) << 8)
            | (c.red as u32 + (r * a) / 255);
    }
    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Abgr(0xff00_0000 | ((b as u32) << 16) | ((g as u32) << 8) | r as u32)
    }
}

// fb_var_screeninfo is 40 u32s: xres, yres, xres_virtual, yres_virtual,
// xoffset, yoffset, ... The driver wants the whole struct back for a pan.
const FBIOGET_VSCREENINFO: libc::c_int = 0x4600;
const FBIOPAN_DISPLAY: libc::c_int = 0x4606;
// _IOW('F', 0x20, __u32) on 32-bit ARM.
const FBIO_WAITFORVSYNC: libc::c_int = 0x4004_4620;

/// One frame period on the 60Hz panel, for the timed fallback.
const FRAME: Duration = Duration::from_micros(16_667);

/// How each drawn frame is held back to the panel's refresh.
///
/// Without this the loop renders back-to-back for as long as anything
/// animates: ~600 frames per five seconds under COUCH_NAV, of which the panel
/// could show 300. Every one of those cost a rasterisation.
#[derive(Copy, Clone, PartialEq, Debug)]
enum Pacing {
    /// FBIO_WAITFORVSYNC: the driver blocks until the next vertical sync.
    WaitForVsync,
    /// FBIOPAN_DISPLAY with a zero offset. Measured on this panel in the LVGL
    /// era: a pan blocks ~17ms waiting for vsync, and an mmap write reaches
    /// the glass on its own, so the pan changes nothing about what is shown -
    /// it is only used as the wait.
    Pan,
    /// Neither ioctl blocks here: sleep until one frame after the draw began.
    Sleep,
}

/// What a frame cost, in microseconds. `work` is rasterising plus the copy to
/// the panel - the cost that would exist at any refresh rate. `wait` is the
/// pacing after it, which is slack, not work.
pub struct FrameCost {
    pub work_us: u64,
    pub wait_us: u64,
}

pub struct Panel {
    fb: File,
    map: &'static mut [u32],
    pub width: u32,
    pub height: u32,
    stride_px: u32,
    ram: Vec<Abgr>,
    /// The screeninfo the driver reported, offsets zeroed, for FBIOPAN_DISPLAY.
    var: Option<[u32; 40]>,
    pacing: Pacing,
}

impl Panel {
    pub fn open() -> std::io::Result<Self> {
        let fb = File::options().read(true).write(true).open("/dev/fb0")?;

        // Ask the driver, not sysfs: /sys/class/graphics/fb0/virtual_size
        // reports the virtual extent (480x2400 here, three pages of
        // scrollback), and rendering to that means drawing three screens per
        // frame. fb_var_screeninfo starts xres, yres, xres_virtual, ...
        let mut vinfo = [0u32; 40];
        let rc = unsafe { libc::ioctl(fb.as_raw_fd(), FBIOGET_VSCREENINFO, vinfo.as_mut_ptr()) };
        let var = (rc == 0 && vinfo[0] > 0 && vinfo[1] > 0).then_some(vinfo);
        let (width, height) = var.map_or((480, 800), |v| (v[0], v[1]));
        let stride = std::fs::read_to_string("/sys/class/graphics/fb0/stride")
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(width * 4);

        let len = (stride * height) as usize;
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE,
                       libc::MAP_SHARED, fb.as_raw_fd(), 0)
        };
        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        let map = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u32, len / 4) };

        let mut panel = Panel {
            fb,
            map,
            width,
            height,
            stride_px: stride / 4,
            ram: vec![Abgr::default(); (width * height) as usize],
            // Page 0 is what is displayed; the pan must say so.
            var: var.map(|mut v| { v[4] = 0; v[5] = 0; v }),
            pacing: Pacing::Sleep,
        };
        panel.pacing = panel.probe_pacing();
        Ok(panel)
    }

    /// Pick the wait once, at startup, and say which.
    ///
    /// Each ioctl is tried twice and the second call timed: the first may
    /// return at once if a sync happens to be due, but the second must block
    /// for a whole period. A driver that accepts the ioctl and returns without
    /// waiting would otherwise pass as vsync and pace nothing.
    ///
    /// COUCH_VSYNC=0 (or `sleep`) forces the timed fallback so the modes can be
    /// compared on the device; `pan` skips FBIO_WAITFORVSYNC; `wait` skips the
    /// pan.
    fn probe_pacing(&mut self) -> Pacing {
        let choice = std::env::var("COUCH_VSYNC").unwrap_or_default();
        let (try_wait, try_pan) = match choice.as_str() {
            "0" | "sleep" => (false, false),
            "pan" => (false, true),
            "wait" => (true, false),
            _ => (true, true),
        };
        let mut why = Vec::new();
        if !try_wait && !try_pan {
            why.push(format!("COUCH_VSYNC={choice}"));
        }

        let fd = self.fb.as_raw_fd();
        if try_wait {
            if let Some(period) = probe("FBIO_WAITFORVSYNC", &mut why, &mut || wait_for_vsync(fd)) {
                println!("couch-gui: pacing: FBIO_WAITFORVSYNC, {:.1}ms per wait{}",
                         period.as_secs_f64() * 1e3, reasons(&why));
                return Pacing::WaitForVsync;
            }
        }
        if try_pan {
            match self.var {
                Some(mut var) => {
                    if let Some(period) =
                        probe("FBIOPAN_DISPLAY", &mut why, &mut || pan_display(fd, &mut var))
                    {
                        println!("couch-gui: pacing: FBIOPAN_DISPLAY, {:.1}ms per wait{}",
                                 period.as_secs_f64() * 1e3, reasons(&why));
                        return Pacing::Pan;
                    }
                }
                None => why.push("FBIOPAN_DISPLAY: no screeninfo".into()),
            }
        }
        println!("couch-gui: pacing: sleep to {:.2}ms{}", FRAME.as_secs_f64() * 1e3, reasons(&why));
        Pacing::Sleep
    }

    /// Take the panel over from fbcon and clear it.
    ///
    /// The marker stops fbcon painting; init's console keeps draining its pipe
    /// but stops drawing, so boot output cannot land on top of the UI. Clearing
    /// matters because the renderer only ever repaints what changed, so
    /// whatever the console left behind would otherwise survive under us.
    pub fn claim(&mut self, background: u32) {
        let _ = std::fs::write("/tmp/couch.gui", b"");
        let bg = 0xff00_0000
            | ((background & 0x0000ff) << 16)
            | (background & 0x00ff00)
            | ((background >> 16) & 0x0000ff);
        for px in self.map.iter_mut() {
            *px = bg;
        }
        for px in self.ram.iter_mut() {
            *px = Abgr(bg);
        }
    }

    /// The panel switches its own backlight off when idle.
    pub fn backlight_on() {
        let _ = std::fs::write("/sys/class/leds/lcd-backlight/brightness", b"255\n");
        let _ = std::fs::write("/sys/class/leds/button-backlight/brightness", b"255\n");
    }

    /// Render one frame if anything changed, then hold until the panel has
    /// had a refresh. Returns what it cost, or None if nothing needed drawing.
    ///
    /// The copy lands first and the wait comes after it, so the copy itself is
    /// not synchronised to blanking - a rectangle can straddle the scan-out.
    /// Copying into the blanking interval instead would need the wait before
    /// the copy, and the region kept across it; acceptable as is for now.
    ///
    /// COUCH_REGION reports what the renderer marked dirty. A frame that costs
    /// far more than its content suggests is almost always claiming a much
    /// larger region than it needs, and that is invisible without this.
    pub fn render(&mut self, window: &MinimalSoftwareWindow) -> Option<FrameCost> {
        let started = Instant::now();
        let report = std::env::var_os("COUCH_REGION").is_some();
        let (w, h, stride_px) = (self.width, self.height, self.stride_px);
        let (ram, map) = (&mut self.ram, &mut *self.map);
        let drew = window.draw_if_needed(|renderer| {
            let region = renderer.render(ram, w as usize);
            if report {
                let (mut n, mut px) = (0u32, 0u64);
                // The geometry, not just the total: a frame that costs far more
                // than the moving element explains is claiming something else,
                // and which rectangle it is names the culprit.
                let mut where_ = String::new();
                for (pos, sz) in region.iter() {
                    n += 1;
                    px += (sz.width * sz.height) as u64;
                    where_.push_str(&format!(
                        " [{},{} {}x{}]", pos.x, pos.y, sz.width, sz.height
                    ));
                }
                println!("couch-gui: region {n} rect(s), {px} px = {}% of screen{where_}",
                         px * 100 / (w as u64 * h as u64));
            }
            // The region's own rectangles, not its bounding box: a change at
            // opposite ends of the screen has a bounding box of nearly the
            // whole panel.
            for (pos, size) in region.iter() {
                let (rx, ry) = (pos.x.max(0) as u32, pos.y.max(0) as u32);
                let n = size.width.min(w.saturating_sub(rx)) as usize;
                if n == 0 {
                    continue;
                }
                for y in ry..(ry + size.height).min(h) {
                    let src = (y * w + rx) as usize;
                    let dst = (y * stride_px + rx) as usize;
                    let s = unsafe {
                        std::slice::from_raw_parts(ram.as_ptr().add(src) as *const u32, n)
                    };
                    map[dst..dst + n].copy_from_slice(s);
                }
            }
        });
        if !drew {
            return None;
        }
        let work = started.elapsed();
        self.pace(started);
        Some(FrameCost {
            work_us: work.as_micros() as u64,
            wait_us: started.elapsed().saturating_sub(work).as_micros() as u64,
        })
    }

    /// Block until the panel has refreshed. An ioctl that worked at startup
    /// and fails now is not retried: the loop must never spin unpaced. A
    /// signal cutting one wait short is not a failure.
    fn pace(&mut self, started: Instant) {
        let fd = self.fb.as_raw_fd();
        let failed = match (self.pacing, self.var.as_mut()) {
            (Pacing::WaitForVsync, _) => wait_for_vsync(fd).err(),
            (Pacing::Pan, Some(var)) => pan_display(fd, var).err(),
            _ => None,
        };
        if let Some(errno) = failed.filter(|e| *e != libc::EINTR) {
            println!("couch-gui: pacing: {:?} failed with {}, sleeping from now on",
                     self.pacing, errno_name(errno));
            self.pacing = Pacing::Sleep;
        }
        if self.pacing == Pacing::Sleep {
            let left = (started + FRAME).saturating_duration_since(Instant::now());
            if !left.is_zero() {
                std::thread::sleep(left);
            }
        }
    }
}

/// Some(period) if the ioctl works and blocks; otherwise the reason it does
/// not is appended to `why`.
fn probe(name: &str, why: &mut Vec<String>, call: &mut dyn FnMut() -> Result<(), i32>) -> Option<Duration> {
    let mut took = Duration::ZERO;
    for _ in 0..2 {
        let t = Instant::now();
        if let Err(errno) = call() {
            why.push(format!("{name}: {}", errno_name(errno)));
            return None;
        }
        took = t.elapsed();
    }
    if took < Duration::from_millis(2) {
        why.push(format!("{name}: returned in {:.2}ms", took.as_secs_f64() * 1e3));
        return None;
    }
    Some(took)
}

fn wait_for_vsync(fd: libc::c_int) -> Result<(), i32> {
    let mut arg: u32 = 0;
    ioctl_result(unsafe { libc::ioctl(fd, FBIO_WAITFORVSYNC, &mut arg as *mut u32) })
}

fn pan_display(fd: libc::c_int, var: &mut [u32; 40]) -> Result<(), i32> {
    ioctl_result(unsafe { libc::ioctl(fd, FBIOPAN_DISPLAY, var.as_mut_ptr()) })
}

fn ioctl_result(rc: libc::c_int) -> Result<(), i32> {
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(-1))
    }
}

fn errno_name(errno: i32) -> String {
    match errno {
        libc::ENOTTY => "ENOTTY".into(),
        libc::EINVAL => "EINVAL".into(),
        libc::ENOSYS => "ENOSYS".into(),
        libc::EPERM => "EPERM".into(),
        libc::EINTR => "EINTR".into(),
        n => format!("errno {n}"),
    }
}

fn reasons(why: &[String]) -> String {
    if why.is_empty() {
        String::new()
    } else {
        format!(" ({})", why.join("; "))
    }
}

pub struct CouchPlatform {
    pub window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl CouchPlatform {
    /// ReusedBuffer, because we hand back the same RAM buffer every frame and
    /// it still holds the previous one - that is what enables partial redraw.
    pub fn install(size: PhysicalSize) -> Result<Rc<MinimalSoftwareWindow>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(size);
        // set_platform reports its own error type, which is not PlatformError.
        slint::platform::set_platform(Box::new(CouchPlatform {
            window: window.clone(),
            start: Instant::now(),
        }))
        .map_err(|e| PlatformError::from(format!("set_platform: {e:?}")))?;
        Ok(window)
    }
}

impl Platform for CouchPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}
