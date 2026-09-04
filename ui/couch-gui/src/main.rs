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

    // Areas are a level above rooms: left and right move between them, up and
    // down between the rooms inside one. Seeded here until a hub daemon exists.
    struct Area {
        name: &'static str,
        rooms: Vec<RoomRow>,
        scenes: Vec<SceneCell>,
    }

    fn room(name: &str, devices: &str, detail: &str, on: i32, glyph: i32) -> RoomRow {
        RoomRow {
            name: name.into(),
            devices: devices.into(),
            detail: detail.into(),
            active_count: on,
            idle: on == 0,
            offline: false,
            dimmed: false,
            glyph,
        }
    }
    fn scene(name: &str) -> SceneCell {
        SceneCell { name: name.into(), active: false }
    }

    let mut areas = vec![
        Area {
            name: "WHOLE HOME",
            rooms: vec![
                room("Living room", "5 devices", "Kodi, Hue, LG C3", 2, 0),
                room("Kitchen", "2 devices", "Sonos Move", 1, 2),
                room("Bedroom", "3 devices", "Hue, Sonos One", 0, 1),
                room("Study", "2 devices", "", 0, 3),
            ],
            scenes: vec![scene("Movie night"), scene("All off")],
        },
        Area {
            name: "UPSTAIRS",
            rooms: vec![
                room("Bedroom", "3 devices", "Hue, Sonos One", 0, 1),
                room("Study", "2 devices", "", 0, 3),
                room("Loft", "1 device", "Hue", 0, 7),
            ],
            scenes: vec![scene("Bedtime"), scene("Upstairs off")],
        },
        Area {
            name: "DOWNSTAIRS",
            rooms: vec![
                room("Living room", "5 devices", "Kodi, Hue, LG C3", 2, 0),
                room("Kitchen", "2 devices", "Sonos Move", 1, 2),
                room("Hallway", "2 devices", "Hue", 0, 4),
            ],
            scenes: vec![scene("Movie night"), scene("Downstairs off")],
        },
        Area {
            name: "OUTSIDE",
            rooms: vec![
                room("Garden", "3 devices", "Hue, Cameras", 1, 6),
                room("Garage", "2 devices", "Hue", 0, 5),
                room("Porch", "1 device", "Hue", 0, 4),
            ],
            scenes: vec![scene("Evening"), scene("Outside off")],
        },
    ];

    // COUCH_ROOMS pads the first area, so the scrolling behaviour is testable
    // without waiting for a house with a dozen rooms in one area.
    if let Ok(n) = std::env::var("COUCH_ROOMS").unwrap_or_default().parse::<usize>() {
        let extra = ["Hallway", "Garage", "Garden", "Loft", "Utility", "Porch", "Cellar"];
        let mut i = 0;
        while areas[0].rooms.len() < n {
            areas[0].rooms.push(room(extra[i % extra.len()], "2 devices", "Hue",
                                     (i % 2) as i32, (i % 4) as i32));
            i += 1;
        }
        areas[0].rooms.truncate(n);
    }

    app.set_activities(ModelRc::new(VecModel::from(vec![
        LiveActivity { kind: 0, title: "Midnight Ferry".into(),
                       source: "SONOS".into(), place: "KITCHEN".into() },
        LiveActivity { kind: 1, title: "Paused - Andrei Rublev".into(),
                       source: "KODI".into(), place: "LIVING".into() },
    ])));
    app.set_area_count(areas.len() as i32);
    app.set_area_dots(ModelRc::new(VecModel::from(vec![true; areas.len()])));

    let areas = std::rc::Rc::new(areas);
    // A page change fills the incoming pane, slides to it, and adopts it when
    // the hub says the animation is done. The pager updates immediately -
    // it does not move, so it can lead rather than lag.
    let pending = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let sliding = std::rc::Rc::new(std::cell::Cell::new(false));

    let put_front = {
        let areas = areas.clone();
        move |app: &App, index: usize| {
            let a = &areas[index];
            app.set_area_name(a.name.into());
            app.set_area_index(index as i32);
            app.set_rooms(ModelRc::new(VecModel::from(a.rooms.clone())));
            app.set_scenes(ModelRc::new(VecModel::from(a.scenes.clone())));
            app.set_focus_index(app.get_activities().row_count() as i32);
            app.invoke_reset_scroll();
        }
    };

    // COUCH_AREA opens on a given page, so each one can be checked without
    // driving the keypad.
    let first = std::env::var("COUCH_AREA")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|i| *i < areas.len())
        .unwrap_or(0);
    put_front(&app, first);
    pending.set(first);

    {
        let weak = app.as_weak();
        let areas = areas.clone();
        let pending = pending.clone();
        let sliding = sliding.clone();
        app.on_area_step(move |delta| {
            let Some(app) = weak.upgrade() else { return };
            // Ignore a second press mid-transition: restarting the timer would
            // strand the first page half way across.
            if sliding.get() {
                return;
            }
            let cur = app.get_area_index();
            let next = (cur + delta).rem_euclid(areas.len() as i32) as usize;
            if next == cur as usize {
                return;
            }
            let a = &areas[next];
            app.set_rooms_next(ModelRc::new(VecModel::from(a.rooms.clone())));
            app.set_scenes_next(ModelRc::new(VecModel::from(a.scenes.clone())));
            app.set_area_name(a.name.into());
            app.set_area_index(next as i32);
            pending.set(next);
            sliding.set(true);
            app.invoke_slide(delta);
        });
    }

    {
        let weak = app.as_weak();
        let pending = pending.clone();
        let sliding = sliding.clone();
        let put_front = put_front.clone();
        app.on_settled(move || {
            if let Some(app) = weak.upgrade() {
                put_front(&app, pending.get());
                sliding.set(false);
            }
        });
    }

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
    // COUCH_SLIDE steps the area every 1.5s, so the transition's real cost can
    // be measured rather than extrapolated from simpler content.
    let auto_slide = std::env::var("COUCH_SLIDE").is_ok();
    let mut slide_at = now_monotonic_us() + 1_500_000;
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
            let n = app.get_rooms().row_count() as i32
                + app.get_activities().row_count() as i32
                + app.get_scenes().row_count() as i32;
            if n > 0 {
                app.set_focus_index((app.get_focus_index() + 1) % n);
            }
            nav_at = now + 700_000;
        }

        if auto_slide && now >= slide_at {
            app.invoke_area_step(1);
            slide_at = now + 1_500_000;
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
