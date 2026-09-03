//! Slint on the HA100 panel, for comparison against couch-gui.
//!
//! Same integration shape as the C app, deliberately: draw into a cached RAM
//! buffer, then copy only the dirty rows into the mapped framebuffer. That is
//! what took the C version from 98ms per frame (a single 1.5MB write) to
//! 0.2ms, and it is the part a toolkit change must not regress.
//!
//! Prints the same statistics couch-gui does so the numbers are comparable.

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{PhysicalSize, SharedString};
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

slint::include_modules!();

/// This panel reads the low byte as red (fb_var_screeninfo says red=0/8), so
/// the buffer is ABGR in memory. Slint lets us say that directly rather than
/// swizzling every pixel afterwards, which is what the C flush_cb has to do.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Default)]
struct Abgr(u32);

impl TargetPixel for Abgr {
    fn blend(&mut self, c: PremultipliedRgbaColor) {
        let a = (255 - c.alpha) as u32;
        let (r, g, b) = (self.0 & 0xff, (self.0 >> 8) & 0xff, (self.0 >> 16) & 0xff);
        let nr = c.red as u32 + (r * a) / 255;
        let ng = c.green as u32 + (g * a) / 255;
        let nb = c.blue as u32 + (b * a) / 255;
        self.0 = 0xff00_0000 | (nb << 16) | (ng << 8) | nr;
    }
    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Abgr(0xff00_0000 | ((b as u32) << 16) | ((g as u32) << 8) | r as u32)
    }
}

struct FbPlatform {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl Platform for FbPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

fn sysfs_u32(path: &str, default: u32) -> u32 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().split(',').next()?.parse().ok())
        .unwrap_or(default)
}

/// evdev: 16-byte input_event on 32-bit ARM (two 4-byte timeval words, then
/// type/code/value). The timestamp is what makes input latency measurable
/// rather than guessed.
struct Keypad(File);

impl Keypad {
    fn open(path: &str) -> Option<Self> {
        let f = File::options().read(true).open(path).ok()?;
        unsafe {
            let fd = f.as_raw_fd();
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        println!("slint-fb: keypad {} open", path);
        Some(Keypad(f))
    }

    fn poll(&mut self) -> Vec<(u16, i32, u64)> {
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 32];
        if let Ok(n) = self.0.read(&mut buf) {
            for ev in buf[..n].chunks_exact(16) {
                let sec = u32::from_ne_bytes(ev[0..4].try_into().unwrap()) as u64;
                let usec = u32::from_ne_bytes(ev[4..8].try_into().unwrap()) as u64;
                let etype = u16::from_ne_bytes(ev[8..10].try_into().unwrap());
                let code = u16::from_ne_bytes(ev[10..12].try_into().unwrap());
                let value = i32::from_ne_bytes(ev[12..16].try_into().unwrap());
                if etype == 1 {
                    out.push((code, value, sec * 1_000_000 + usec));
                }
            }
        }
        out
    }
}

fn now_us() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000
}

/// evdev stamps events against CLOCK_REALTIME, so input latency has to be
/// measured on the same clock. Subtracting an epoch timestamp from a monotonic
/// uptime saturates to zero, which reads as "no latency" rather than as a bug.
fn now_realtime_us() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000
}

fn key_for(code: u16) -> Option<slint::platform::Key> {
    match code {
        103 => Some(slint::platform::Key::UpArrow),    // KEY_UP
        108 => Some(slint::platform::Key::DownArrow),  // KEY_DOWN
        28 | 96 => Some(slint::platform::Key::Return), // KEY_ENTER / KP_ENTER
        _ => None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fb = File::options().read(true).write(true).open("/dev/fb0")?;

    // Ask the driver, not sysfs. /sys/class/graphics/fb0/virtual_size reports
    // the *virtual* extent - 480x2400 here, three pages of scrollback - and
    // rendering to that means drawing three screens for every frame.
    // fb_var_screeninfo starts with xres, yres, xres_virtual, yres_virtual.
    const FBIOGET_VSCREENINFO: libc::c_int = 0x4600;   /* musl on armv7 types the request as int */
    let mut vinfo = [0u32; 40];
    let rc = unsafe { libc::ioctl(fb.as_raw_fd(), FBIOGET_VSCREENINFO, vinfo.as_mut_ptr()) };
    let (w, h) = if rc == 0 && vinfo[0] > 0 && vinfo[1] > 0 {
        (vinfo[0], vinfo[1])
    } else {
        (480, 800)
    };
    let stride = sysfs_u32("/sys/class/graphics/fb0/stride", w * 4);
    println!("slint-fb: {}x{} (virtual {}x{}) stride {}", w, h, vinfo[2], vinfo[3], stride);
    let len = (stride * h) as usize;
    let map = unsafe {
        libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE,
                   libc::MAP_SHARED, fb.as_raw_fd(), 0)
    };
    if map == libc::MAP_FAILED {
        return Err("mmap /dev/fb0 failed".into());
    }
    let fb_px = unsafe { std::slice::from_raw_parts_mut(map as *mut u32, len / 4) };

    // Backlight: the panel switches itself off when idle.
    let _ = std::fs::write("/sys/class/leds/lcd-backlight/brightness", "255\n");

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(FbPlatform {
        window: window.clone(),
        start: Instant::now(),
    }))
    .map_err(|e| format!("set_platform: {e:?}"))?;
    window.set_size(PhysicalSize::new(w, h));

    let app = App::new().map_err(|e| format!("App::new: {e:?}"))?;
    app.set_clock(SharedString::from("9:31 PM"));
    app.set_battery(SharedString::from("100%"));
    app.show().map_err(|e| format!("show: {e:?}"))?;

    let mut ram = vec![Abgr::default(); (w * h) as usize];
    let mut pads: Vec<Keypad> = ["/dev/input/event1", "/dev/input/event2"]
        .iter().filter_map(|p| Keypad::open(p)).collect();
    println!("slint-fb: {} keypad device(s), entering loop", pads.len());

    // Same timings as couch-gui: the keypad has autorepeat disabled in the
    // device tree, so holding a key repeats only because we make it.
    const REPEAT_DELAY_MS: u64 = 400;
    const REPEAT_RATE_MS: u64 = 70;
    let (mut held, mut held_since, mut last_repeat) = (0u16, 0u64, 0u64);

    let (mut frames, mut render_us, mut copy_us) = (0u64, 0u64, 0u64);
    let (mut in_n, mut in_sum, mut in_max) = (0u64, 0u64, 0u64);
    let mut last_stat = now_us();

    loop {
        let mut pressed = None;
        for pad in pads.iter_mut() {
            for (code, value, stamp) in pad.poll() {
                if value == 0 {
                    if code == held { held = 0; }
                    continue;
                }
                if value != 1 { continue; }
                let lat = now_realtime_us().saturating_sub(stamp);
                in_n += 1; in_sum += lat; in_max = in_max.max(lat);
                held = code;
                held_since = now_us();
                last_repeat = 0;
                pressed = key_for(code).or(pressed);
            }
        }
        if held != 0 {
            let now = now_us();
            let due = if last_repeat == 0 {
                held_since + REPEAT_DELAY_MS * 1000
            } else {
                last_repeat + REPEAT_RATE_MS * 1000
            };
            if now >= due {
                last_repeat = now;
                pressed = key_for(held).or(pressed);
            }
        }
        if let Some(key) = pressed {
            let text = SharedString::from(char::from(key));
            window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            window.dispatch_event(WindowEvent::KeyReleased { text });
        }

        slint::platform::update_timers_and_animations();

        let t0 = now_us();
        let drawn = window.draw_if_needed(|renderer| {
            let region = renderer.render(&mut ram, w as usize);
            let t1 = now_us();
            // Copy only what changed, row by row, exactly like couch-gui.
            let (rx, ry) = (region.bounding_box_origin().x.max(0) as u32,
                            region.bounding_box_origin().y.max(0) as u32);
            let size = region.bounding_box_size();
            let (rw, rh) = (size.width, size.height);
            for y in ry..(ry + rh).min(h) {
                let src = (y * w + rx) as usize;
                let dst = (y * (stride / 4) + rx) as usize;
                let n = (rw.min(w - rx)) as usize;
                let s = unsafe {
                    std::slice::from_raw_parts(ram.as_ptr().add(src) as *const u32, n)
                };
                fb_px[dst..dst + n].copy_from_slice(s);
            }
            render_us += t1 - t0;
            copy_us += now_us() - t1;
            frames += 1;
        });
        if !drawn {
            std::thread::sleep(Duration::from_millis(5));
        }

        if now_us() - last_stat > 5_000_000 && frames > 0 {
            println!(
                "slint-fb: {} frames, {} us/frame render, {} us/frame copy | input {} us avg, {} us max (n={})",
                frames, render_us / frames, copy_us / frames,
                if in_n > 0 { in_sum / in_n } else { 0 }, in_max, in_n
            );
            last_stat = now_us();
            frames = 0; render_us = 0; copy_us = 0;
        }
    }
}
