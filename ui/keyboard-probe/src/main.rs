//! Instantiates the probe UI and renders one frame, so that nothing in it can
//! be dropped as dead code and the two binaries differ only by the component.

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::PhysicalSize;

slint::include_modules!();

struct Probe565 {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl Platform for Probe565 {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(PhysicalSize::new(480, 800));
    slint::platform::set_platform(Box::new(Probe565 { window: window.clone(), start: Instant::now() }))
        .map_err(|e| format!("set_platform: {e:?}"))?;

    // Slint's PlatformError is not a std::error::Error without the std
    // feature, which is the same reason couch-gui's main stringifies these.
    let ui = Probe::new().map_err(|e| format!("Probe::new: {e:?}"))?;
    ui.show().map_err(|e| format!("show: {e:?}"))?;

    let mut buf = vec![Rgb565Pixel(0); 480 * 800];
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buf, 480);
    });
    println!("{}", buf.len());
    Ok(())
}
