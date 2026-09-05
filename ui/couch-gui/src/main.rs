//! The Couch remote's UI.
//!
//! The screens themselves are still being designed; everything here is the
//! platform underneath them - panel, input, and the device state the UI shows.

mod keypad;
mod mic;
mod panel;
mod qr;
mod system;
mod touch;

use std::time::Duration;

use slint::platform::{PointerEventButton, WindowEvent};
use slint::LogicalPosition;
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
        activities: Vec<LiveActivity>,
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
    fn scene_on(name: &str) -> SceneCell {
        SceneCell { name: name.into(), active: true }
    }
    // kind 0 is audio (bars), 1 is video (play triangle).
    fn act(kind: i32, title: &str, source: &str, place: &str) -> LiveActivity {
        LiveActivity {
            kind,
            title: title.into(),
            source: source.into(),
            place: place.into(),
        }
    }

    let mut areas = vec![
        Area {
            name: "WHOLE HOME",
            activities: vec![
                act(0, "Midnight Ferry", "SONOS", "KITCHEN"),
                act(1, "Paused - Andrei Rublev", "KODI", "LIVING"),
                act(0, "Radio Paradise", "SONOS", "STUDY"),
            ],
            rooms: vec![
                room("Living room", "5 devices", "Kodi, Hue, LG C3", 2, 0),
                room("Kitchen", "2 devices", "Sonos Move", 1, 2),
                room("Bedroom", "3 devices", "Hue, Sonos One", 0, 1),
                room("Study", "2 devices", "", 0, 3),
            ],
            scenes: vec![
                scene("Movie night"),
                scene("Good morning"),
                scene_on("Away"),
                scene("Dinner"),
                scene("All off"),
            ],
        },
        Area {
            name: "UPSTAIRS",
            activities: vec![act(0, "White noise", "SONOS", "BEDROOM")],
            rooms: vec![
                room("Bedroom", "3 devices", "Hue, Sonos One", 0, 1),
                room("Study", "2 devices", "", 0, 3),
                room("Loft", "1 device", "Hue", 0, 7),
            ],
            scenes: vec![scene("Bedtime"), scene("Wake up"), scene("Upstairs off")],
        },
        Area {
            name: "DOWNSTAIRS",
            activities: vec![
                act(1, "Paused - Andrei Rublev", "KODI", "LIVING"),
                act(0, "Midnight Ferry", "SONOS", "KITCHEN"),
                act(1, "Front door", "CAMERA", "HALLWAY"),
                act(0, "The Rest Is History", "SONOS", "LIVING"),
                act(1, "Formula 1 - Practice 2", "PLEX", "LIVING"),
            ],
            rooms: vec![
                room("Living room", "5 devices", "Kodi, Hue, LG C3", 2, 0),
                room("Kitchen", "2 devices", "Sonos Move", 1, 2),
                room("Hallway", "2 devices", "Hue", 0, 4),
            ],
            scenes: vec![
                scene_on("Movie night"),
                scene("Cooking"),
                scene("Downstairs off"),
            ],
        },
        Area {
            name: "OUTSIDE",
            activities: vec![],
            rooms: vec![
                room("Garden", "3 devices", "Hue, Cameras", 1, 6),
                room("Garage", "2 devices", "Hue", 0, 5),
                room("Porch", "1 device", "Hue", 0, 4),
            ],
            scenes: vec![
                scene("Evening"),
                scene("Security on"),
                scene("Watering"),
                scene("Outside off"),
            ],
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
            // Before anything else: the row indices are about to mean something
            // different, and the ring must not animate through the difference.
            app.invoke_begin_swap();
            app.set_area_name(a.name.into());
            app.set_area_index(index as i32);
            app.set_activities(ModelRc::new(VecModel::from(a.activities.clone())));
            app.set_rooms(ModelRc::new(VecModel::from(a.rooms.clone())));
            app.set_scenes(ModelRc::new(VecModel::from(a.scenes.clone())));
            app.set_focus_row(if a.activities.is_empty() { 0 } else { 1 });
            app.invoke_reset_scroll();
            // The page is whole; the ring may animate again.
            app.invoke_end_swap();
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

    // COUCH_FOCUS parks focus on a row - 0 is the activity strip, then one per
    // room, then the scenes row - so any focus position can be photographed
    // without driving the keypad.
    if let Ok(n) = std::env::var("COUCH_FOCUS").unwrap_or_default().parse::<i32>() {
        app.set_focus_row(n);
        app.invoke_settle_focus();
    }

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
            app.set_activities_next(ModelRc::new(VecModel::from(a.activities.clone())));
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

    // OK on the strip or the scenes row opens what is there: straight through
    // when there is one of it, as a list when there are several. Anywhere else
    // it is a room, which has no screen behind it yet.
    {
        let weak = app.as_weak();
        let areas = areas.clone();
        let pending = pending.clone();
        app.on_activated(move |row| {
            let Some(app) = weak.upgrade() else { return };
            let a = &areas[pending.get()];
            let first_room = if a.activities.is_empty() { 0 } else { 1 };
            let scenes_row = first_room + a.rooms.len() as i32;

            if row < first_room {
                if a.activities.len() == 1 {
                    println!("couch-gui: open activity '{}'", a.activities[0].title);
                    return;
                }
                let items: Vec<ChoiceItem> = a
                    .activities
                    .iter()
                    .map(|x| ChoiceItem {
                        title: x.title.clone(),
                        detail: SharedString::from(format!("{} · {}", x.source, x.place)),
                        // Everything in this list is playing, so a dot marking
                        // that would be on every row and mean nothing.
                        active: false,
                    })
                    .collect();
                app.set_chooser_title("NOW PLAYING".into());
                app.set_chooser_items(ModelRc::new(VecModel::from(items)));
                app.set_chooser_index(0);
                app.set_chooser_shown(true);
            } else if row == scenes_row {
                if a.scenes.len() == 1 {
                    println!("couch-gui: run scene '{}'", a.scenes[0].name);
                    return;
                }
                let items: Vec<ChoiceItem> = a
                    .scenes
                    .iter()
                    .map(|x| ChoiceItem {
                        title: x.name.clone(),
                        detail: SharedString::new(),
                        active: x.active,
                    })
                    .collect();
                app.set_chooser_title("SCENES".into());
                app.set_chooser_items(ModelRc::new(VecModel::from(items)));
                app.set_chooser_index(0);
                app.set_chooser_shown(true);
            } else {
                let room = (row - first_room) as usize;
                if let Some(r) = a.rooms.get(room) {
                    println!("couch-gui: open room '{}'", r.name);
                }
            }
        });
    }

    {
        let weak = app.as_weak();
        app.on_chosen(move |index| {
            let Some(app) = weak.upgrade() else { return };
            let title = app
                .get_chooser_items()
                .row_data(index as usize)
                .map(|i| i.title.to_string())
                .unwrap_or_default();
            println!("couch-gui: chose '{title}'");
            app.set_chooser_shown(false);
        });
    }
    app.on_back(|| println!("couch-gui: back"));
    app.on_home(|| println!("couch-gui: home"));

    app.show().map_err(|e| format!("show: {e:?}"))?;

    let mut pad = Keypad::open();
    println!("couch-gui: {} keypad device(s)", pad.device_count());

    // The panel is a touchscreen too. Slint does the hit testing once the
    // pointer events are fed in, so this is only a translation layer.
    let mut mic = mic::Mic::new();
    // Push to talk, with a latch. Hold the key and it records while held;
    // tap it and it stays on until the next tap. The button is small and the
    // thing being dictated is a sentence, so insisting on a hold would be a
    // worse remote - but a hold is what a hand does without being told, and
    // both have to mean the obvious thing.
    let mut mic_down_at = 0u64;
    let mut mic_latched = false;
    const LATCH_UNDER_US: u64 = 600_000;
    let mut pointer = touch::Touch::open();
    println!(
        "couch-gui: touchscreen {}",
        if pointer.present() { "present" } else { "absent" }
    );

    // Cached because reading it spawns a process; the offset only changes when
    // the timezone does.
    let mut tz_offset = system::utc_offset_seconds();
    let mut tz_checked = now_monotonic_us();
    // COUCH_OPEN presses OK on whatever COUCH_FOCUS selected, so the chooser
    // can be photographed without driving the keypad. Fired from the loop a
    // moment in rather than here: a property set before the first frame has
    // nothing to animate from, so opening it at startup would never show the
    // transition it exists to demonstrate.
    let open_at = std::env::var("COUCH_OPEN")
        .is_ok()
        .then(|| now_monotonic_us() + 2_000_000);
    let mut opened = false;

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
            // Hold to talk. The key is not routed into the UI: it opens the
            // microphone and nothing else, so there is no screen on which it
            // means something different.
            match press.mic {
                Some(true) => {
                    if mic_latched {
                        mic_latched = false;
                        mic.stop();
                    } else {
                        mic_down_at = now_monotonic_us();
                        mic.start();
                    }
                }
                Some(false) => {
                    if mic.recording() {
                        if now_monotonic_us() - mic_down_at < LATCH_UNDER_US {
                            mic_latched = true;
                        } else {
                            mic.stop();
                        }
                    }
                }
                None => {}
            }
            if let Some(key) = press.key {
                let text = SharedString::from(char::from(key));
                window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                window.dispatch_event(WindowEvent::KeyReleased { text });
            }
        }

        // Slint's own hit testing decides what a tap lands on, so nothing here
        // needs to know what is on screen.
        while let Some(event) = pointer.poll() {
            let (position, ev) = match event {
                touch::Event::Pressed { x, y } => (
                    LogicalPosition::new(x, y),
                    0,
                ),
                touch::Event::Moved { x, y } => (LogicalPosition::new(x, y), 1),
                touch::Event::Released { x, y } => (LogicalPosition::new(x, y), 2),
            };
            window.dispatch_event(match ev {
                0 => WindowEvent::PointerPressed { position, button: PointerEventButton::Left },
                1 => WindowEvent::PointerMoved { position },
                _ => WindowEvent::PointerReleased { position, button: PointerEventButton::Left },
            });
        }

        // The meter has to keep up with a voice, so it is read every frame
        // rather than on the one-second tick.
        if app.get_recording() != mic.recording() {
            app.set_recording(mic.recording());
        }
        if mic.recording() {
            app.set_mic_level(mic.meter());
        } else if mic_latched {
            // The capture hit its own limit while latched; the latch must not
            // outlive it or the next press would only clear a flag.
            mic_latched = false;
        }
        if app.get_mic_latched() != mic_latched {
            app.set_mic_latched(mic_latched);
        }

        let now = now_monotonic_us();
        if let Some(at) = open_at {
            if !opened && now >= at {
                opened = true;
                app.invoke_activated(app.get_focus_row());
            }
        }

        // Device state changes in seconds, not frames.
        if now - last_tick > 1_000_000 {
            last_tick = now;
            if now - tz_checked > 3_600_000_000 {
                tz_offset = system::utc_offset_seconds();
                tz_checked = now;
            }
            app.set_clock(SharedString::from(system::clock_24h(tz_offset)));

            // The deadline is the daemon's, carried in the file, so restarting
            // this process cannot extend a PIN that is already on its way out.
            match system::pairing_pin() {
                Some((pin, left)) => {
                    app.set_pair_pin(SharedString::from(pin));
                    app.set_pair_seconds(left);
                    app.set_pair_shown(left > 0);
                }
                None => app.set_pair_shown(false),
            }
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
            let rows = app.get_rooms().row_count() as i32
                + if app.get_activities().row_count() > 0 { 1 } else { 0 }
                + 1;
            if rows > 0 {
                app.set_focus_row((app.get_focus_row() + 1) % rows);
                    app.invoke_settle_focus();
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
