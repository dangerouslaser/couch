//! The Couch remote's UI.
//!
//! The screens themselves are still being designed; everything here is the
//! platform underneath them - panel, input, and the device state the UI shows.

mod keypad;
mod panel;
mod qr;
mod system;

use std::time::Duration;

use slint::platform::WindowEvent;
use slint::{PhysicalSize, SharedString};

use keypad::{now_monotonic_us, Keypad};
use panel::{CouchPlatform, Panel};
use system::Approval;

slint::include_modules!();

const BACKGROUND: u32 = 0x09090b;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut screen = Panel::open()?;
    println!("couch-gui: panel {}x{}", screen.width, screen.height);

    let window = CouchPlatform::install(PhysicalSize::new(screen.width, screen.height))
        .map_err(|e| format!("platform: {e:?}"))?;

    Panel::backlight_on();
    screen.claim(BACKGROUND);

    let app = App::new().map_err(|e| format!("App::new: {e:?}"))?;
    app.show().map_err(|e| format!("show: {e:?}"))?;

    let mut pad = Keypad::open();
    println!("couch-gui: {} keypad device(s)", pad.device_count());

    // Cached because reading it spawns a process; the offset only changes when
    // the timezone does.
    let mut tz_offset = system::utc_offset_seconds();
    let mut tz_checked = now_monotonic_us();
    let mut last_tick = 0u64;
    let mut last_setup: Option<bool> = None;

    let (mut frames, mut render_us, mut frame_max) = (0u64, 0u64, 0u64);
    let (mut in_n, mut in_sum, mut in_max) = (0u64, 0u64, 0u64);
    let mut last_stat = now_monotonic_us();

    loop {
        if let Some(press) = pad.poll() {
            if !press.repeat {
                in_n += 1;
                in_sum += press.latency_us;
                in_max = in_max.max(press.latency_us);
            }
            if let Some(key) = press.key {
                let text = SharedString::from(char::from(key));
                window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                window.dispatch_event(WindowEvent::KeyReleased { text });
            }
        }

        let now = now_monotonic_us();

        // Device state changes in seconds, not frames.
        if now - last_tick > 1_000_000 {
            last_tick = now;
            if now - tz_checked > 3_600_000_000 {
                tz_offset = system::utc_offset_seconds();
                tz_checked = now;
            }
            app.set_clock(SharedString::from(system::clock_string(tz_offset)));
            match system::battery() {
                Some(b) => {
                    app.set_battery(SharedString::from(format!("{}%", b.percent)));
                    app.set_charging(b.charging);
                }
                None => app.set_battery(SharedString::default()),
            }

            let setup = system::in_setup_mode();
            if last_setup != Some(setup) {
                last_setup = Some(setup);
                app.set_setup_mode(setup);
                if setup {
                    let ssid = system::setup_ssid();
                    app.set_ssid(SharedString::from(ssid.clone()));
                    if let Some(img) = qr::render(&qr::wifi_join_record(&ssid), 288) {
                        app.set_qr(img);
                    }
                }
            }
            app.set_approval(match system::approval_state() {
                Approval::Idle => 0,
                Approval::Waiting => 1,
                Approval::Granted => 2,
                Approval::TimedOut => 3,
            });
        }

        slint::platform::update_timers_and_animations();

        let t0 = now_monotonic_us();
        if screen.render(&window) {
            let d = now_monotonic_us() - t0;
            render_us += d;
            frames += 1;
            frame_max = frame_max.max(d);
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }

        if now_monotonic_us() - last_stat > 5_000_000 && frames > 0 {
            println!(
                "couch-gui: {frames} frames, {} us/frame, {frame_max} us worst | input {} us avg, {in_max} us max (n={in_n})",
                render_us / frames,
                if in_n > 0 { in_sum / in_n } else { 0 }
            );
            last_stat = now_monotonic_us();
            frames = 0;
            render_us = 0;
            frame_max = 0;
            in_n = 0;
            in_sum = 0;
            in_max = 0;
        }
    }
}
