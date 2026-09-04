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
use slint::{Model, ModelRc, PhysicalSize, SharedString, VecModel};

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

    // Reference content from the design handoff, so the hub renders as designed
    // until a hub daemon exists to supply it. Areas come from local config in
    // the real thing, which is why they are present before any bridge answers.
    let activities_shown = true;
    app.set_activities(ModelRc::new(VecModel::from(vec![
        LiveActivity { kind: 0, title: "Midnight Ferry".into(),
                       source: "SONOS".into(), place: "KITCHEN".into() },
        LiveActivity { kind: 1, title: "Paused - Andrei Rublev".into(),
                       source: "KODI".into(), place: "LIVING".into() },
    ])));
    // COUCH_AREAS overrides the count so the overflow behaviour is testable:
    // the hub is designed for four and must never scroll, and what a fifth or
    // eighth area does to it is an open question in the handoff.
    let mut areas = vec![
        AreaRow { name: "Living room".into(), devices: "5 devices".into(),
                  detail: "Kodi, Hue, LG C3".into(),
                  active_count: 2, idle: false, offline: false, dimmed: false, glyph: 0 },
        AreaRow { name: "Bedroom".into(), devices: "3 devices".into(),
                  detail: "Hue, Sonos One".into(),
                  active_count: 0, idle: true, offline: false, dimmed: false, glyph: 1 },
        AreaRow { name: "Kitchen".into(), devices: "2 devices".into(),
                  detail: "Sonos Move".into(),
                  active_count: 1, idle: false, offline: false, dimmed: false, glyph: 2 },
        AreaRow { name: "Study".into(), devices: "2 devices".into(), detail: "".into(),
                  active_count: 0, idle: true, offline: false, dimmed: true, glyph: 3 },
    ];
    if let Ok(n) = std::env::var("COUCH_AREAS").unwrap_or_default().parse::<usize>() {
        let extra = ["Hallway", "Garage", "Garden", "Loft", "Utility", "Porch", "Cellar"];
        while areas.len() > n {
            areas.pop();
        }
        let mut i = 0;
        while areas.len() < n {
            let name = extra[i % extra.len()];
            areas.push(AreaRow {
                name: name.into(),
                devices: "2 devices".into(),
                detail: "Hue".into(),
                active_count: (i % 2) as i32,
                idle: i % 2 == 1,
                offline: false,
                dimmed: false,
                glyph: (i % 4) as i32,
            });
            i += 1;
        }
    }
    // What fits, measured against the 800px the hub may never exceed:
    // 704px of content once the status bar and padding are taken, less the
    // labels, the scene row, and - when anything is playing - the activity
    // strip. That leaves room for four rows, or five when the strip is hidden.
    // Anything past that collapses into a single "More areas" stop, so the
    // hub's height is the same whether a home has four areas or forty.
    let capacity = if activities_shown { 4 } else { 5 };
    let more = areas.len().saturating_sub(capacity);
    if more > 0 {
        // The overflow row occupies a slot of its own.
        areas.truncate(capacity - 1);
    }
    app.set_more_count((if more > 0 { more + 1 } else { 0 }) as i32);
    app.set_areas(ModelRc::new(VecModel::from(areas)));

    app.set_scenes(ModelRc::new(VecModel::from(vec![
        SceneCell { name: "Movie night".into(), active: false },
        SceneCell { name: "All off".into(), active: false },
    ])));

    app.on_activated(|index| println!("couch-gui: activated stop {index}"));
    app.on_back(|| println!("couch-gui: back"));
    app.on_home(|| println!("couch-gui: home"));

    app.show().map_err(|e| format!("show: {e:?}"))?;

    let mut pad = Keypad::open();
    println!("couch-gui: {} keypad device(s)", pad.device_count());

    // Cached because reading it spawns a process; the offset only changes when
    // the timezone does.
    let mut tz_offset = system::utc_offset_seconds();
    let mut tz_checked = now_monotonic_us();
    let mut last_tick = 0u64;
    let mut last_setup: Option<bool> = None;
    // COUCH_NAV walks the focus ring on a timer, so its repaint behaviour is
    // observable without someone pressing buttons.
    let nav = std::env::var("COUCH_NAV").is_ok();
    let mut nav_at = now_monotonic_us() + 700_000;

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
            app.set_clock(SharedString::from(system::clock_24h(tz_offset)));
            match system::battery() {
                Some(b) => {
                    app.set_battery(b.percent);
                    app.set_charging(b.charging);
                }
                None => app.set_battery(0),
            }

            let setup = system::in_setup_mode();
            if last_setup != Some(setup) {
                last_setup = Some(setup);
                app.set_setup_mode(setup);
                app.set_context_title(SharedString::from(if setup { "SETUP" } else { "HOME" }));
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

        if nav && now >= nav_at {
            let n = app.get_areas().row_count() as i32
                + app.get_activities().row_count() as i32
                + app.get_scenes().row_count() as i32;
            if n > 0 {
                app.set_focus_index((app.get_focus_index() + 1) % n);
            }
            nav_at = now + 700_000;
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
