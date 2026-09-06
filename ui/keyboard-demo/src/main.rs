//! A harness for the on-screen keyboard, on a desktop.
//!
//! Two ways to run it, and the second is the one that answers questions:
//!
//! - no arguments opens a 480x800 window with the panel's status bar and the
//!   keyboard under it. Arrow keys, Enter and Escape are the remote's six keys;
//!   the mouse stands in for the digitizer.
//!
//! - `--shoot DIR --script ...` drives the same UI headlessly through a
//!   scripted sequence of key presses and writes a PNG at each `shot:` marker.
//!   The clock is virtual, so animations settle in zero real time and the
//!   frames are identical from one run to the next.
//!
//! Both use the software renderer. Femtovg would anti-alias and lay out text
//! differently, which would make the screenshots a picture of something other
//! than the device.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::platform::PointerEventButton;
use slint::{LogicalPosition, PhysicalSize, Rgb8Pixel, SharedString};

slint::include_modules!();

const W: u32 = 480;
const H: u32 = 800;

/// A platform whose clock only moves when the script says so: an animation that
/// takes 160ms of wall time takes none here, and every run produces the same
/// pixels.
struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    now: Rc<Cell<Duration>>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.now.get()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    };

    let seed = Options {
        masked: !args.iter().any(|a| a == "--plain"),
        title: flag("--title").unwrap_or_else(|| "WI-FI PASSWORD".into()),
        placeholder: flag("--placeholder").unwrap_or_else(|| "Passphrase".into()),
        text: flag("--text").unwrap_or_default(),
    };

    match flag("--shoot") {
        Some(dir) => shoot(&dir, &flag("--script").unwrap_or_default(), &seed),
        None => interactive(&seed),
    }
}

struct Options {
    masked: bool,
    title: String,
    placeholder: String,
    text: String,
}

fn apply(demo: &Demo, o: &Options) {
    demo.set_masked(o.masked);
    demo.set_title_text(SharedString::from(o.title.as_str()));
    demo.set_placeholder_text(SharedString::from(o.placeholder.as_str()));
    demo.set_value(SharedString::from(o.text.as_str()));
}

fn interactive(o: &Options) -> Result<(), Box<dyn std::error::Error>> {
    let demo = Demo::new()?;
    apply(&demo, o);
    demo.on_accepted(|t| println!("accepted: {t:?}"));
    demo.on_cancelled(|| {
        println!("cancelled");
        let _ = slint::quit_event_loop();
    });
    demo.run()?;
    println!("final text: {:?}", demo.get_value());
    Ok(())
}

/// One script token: a key to press, a place to tap, or a frame to write out.
enum Step<'a> {
    Press(&'a str, char),
    Tap(f32, f32),
    Shot(&'a str),
}

fn parse<'a>(script: &'a str) -> Result<Vec<Step<'a>>, String> {
    script
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            if let Some(name) = t.strip_prefix("shot:") {
                return Ok(Step::Shot(name));
            }
            // A tap stands in for the digitizer, which is the only way to
            // exercise the pointer path without the hardware answering the
            // question of whether it emits events at all.
            if let Some(at) = t.strip_prefix("tap:") {
                let (x, y) = at.split_once(':').ok_or_else(|| format!("bad tap {t:?}"))?;
                return Ok(Step::Tap(
                    x.parse().map_err(|_| format!("bad tap {t:?}"))?,
                    y.parse().map_err(|_| format!("bad tap {t:?}"))?,
                ));
            }
            Ok(Step::Press(t, match t {
                "u" => Key::UpArrow.into(),
                "d" => Key::DownArrow.into(),
                "l" => Key::LeftArrow.into(),
                "r" => Key::RightArrow.into(),
                "ok" => Key::Return.into(),
                "back" => Key::Escape.into(),
                "home" => Key::Home.into(),
                other => return Err(format!("unknown script token {other:?}")),
            }))
        })
        .collect()
}

fn shoot(dir: &str, script: &str, o: &Options) -> Result<(), Box<dyn std::error::Error>> {
    let steps = parse(script)?;
    std::fs::create_dir_all(dir)?;

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(PhysicalSize::new(W, H));
    let now = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(Headless { window: window.clone(), now: now.clone() }))
        .map_err(|e| format!("set_platform: {e:?}"))?;

    let demo = Demo::new()?;
    apply(&demo, o);
    demo.on_accepted(|t| println!("accepted: {t:?}"));
    demo.on_cancelled(|| println!("cancelled"));
    demo.show()?;

    let mut buf = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (W * H) as usize];
    // Settle the first frame before anything is pressed, so the region reported
    // for step one is a step and not the initial paint.
    settle(&window, &now, &mut buf, None);

    for step in steps {
        match step {
            Step::Press(token, k) => {
                let text = SharedString::from(k);
                window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                window.dispatch_event(WindowEvent::KeyReleased { text });
                settle(&window, &now, &mut buf, Some(token));
            }
            Step::Tap(x, y) => {
                let position = LogicalPosition::new(x, y);
                let button = PointerEventButton::Left;
                window.dispatch_event(WindowEvent::PointerMoved { position });
                window.dispatch_event(WindowEvent::PointerPressed { position, button });
                settle(&window, &now, &mut buf, Some("tap down"));
                window.dispatch_event(WindowEvent::PointerReleased { position, button });
                settle(&window, &now, &mut buf, Some("tap up"));
            }
            Step::Shot(name) => {
                settle(&window, &now, &mut buf, None);
                let path = format!("{dir}/{name}.png");
                write_png(&path, &buf)?;
                println!("wrote {path}  text={:?}", demo.get_value());
            }
        }
    }
    Ok(())
}

/// Advance the virtual clock past the longest animation and draw until nothing
/// is dirty, reporting what the renderer claimed - the same number
/// `COUCH_REGION=1` prints on the device, which is the number that decides
/// whether a frame is affordable.
fn settle(
    window: &Rc<MinimalSoftwareWindow>,
    now: &Rc<Cell<Duration>>,
    buf: &mut [Rgb8Pixel],
    report: Option<&str>,
) {
    let mut first = true;
    for _ in 0..40 {
        slint::platform::update_timers_and_animations();
        let drew = window.draw_if_needed(|renderer| {
            let region = renderer.render(buf, W as usize);
            if first {
                if let Some(name) = report {
                    let mut px = 0u64;
                    let mut where_ = String::new();
                    for (pos, size) in region.iter() {
                        px += (size.width * size.height) as u64;
                        where_ += &format!(" [{},{} {}x{}]", pos.x, pos.y, size.width, size.height);
                    }
                    println!(
                        "{name}: dirty {px} px = {}% of panel{where_}",
                        px * 100 / (W as u64 * H as u64)
                    );
                }
            }
            first = false;
        });
        now.set(now.get() + Duration::from_millis(20));
        if !drew && !slint::platform::duration_until_next_timer_update()
            .is_some_and(|d| d < Duration::from_millis(200))
        {
            break;
        }
    }
}

fn write_png(path: &str, buf: &[Rgb8Pixel]) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::with_capacity(buf.len() * 3);
    for px in buf {
        bytes.extend_from_slice(&[px.r, px.g, px.b]);
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&bytes)?;
    Ok(())
}
