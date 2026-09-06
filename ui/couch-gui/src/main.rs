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

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::LogicalPosition;
use slint::{Model, ModelRc, PhysicalSize, SharedString, VecModel};

use keypad::{now_monotonic_us, Keypad};
use panel::{Arrive, CouchPlatform, Panel, SlideCost, SLIDE};
use system::Approval;

slint::include_modules!();

const BACKGROUND: u32 = 0x09090b;

/// A page change the UI asked for. The callback that asked records it and
/// nothing else; the loop performs it once the event that caused it has been
/// dispatched. A transition renders a frame and draws to the panel, and the
/// window's `draw_if_needed` cannot be re-entered from inside a Slint
/// callback - so a callback only ever says what it wants, and the ways the
/// chooser closes from inside app.slint are callbacks too, for the same
/// reason.
#[derive(Copy, Clone, Debug)]
enum Intent {
    /// Step the area by this many, wrapping; the sign is the direction.
    Area(i32),
    /// Show the chooser. Its content is already set by the time this is asked.
    OpenChooser,
    CloseChooser,
}

/// How much of the panel is on.
///
/// Three levels, two timers. Dimmed, the screen is still readable and the
/// first key acts as it always would; off, the panel is powered down (LCM,
/// backlight PWM and the touch controller all suspended by the driver) and
/// the first key only wakes it - a dark remote should not change the house
/// because someone found the wrong button in the dark. The microphone key is
/// the exception: holding it in the dark means "talk", so it wakes and records.
/// Only a key wakes; a touch on a dimmed or dark panel is ignored, and the
/// touch controller is suspended anyway while the panel is off.
///
/// A pairing PIN on screen, a recording in progress, or first-run setup hold
/// the panel awake: each is something a person is looking at or waiting on.
///
/// A second after every wake the backlight is written once more, forced past
/// the LED layer's deduplication - see `Panel::set_backlight` for the dropped
/// write that left the panel stuck dim while the LED node said 255.
#[derive(Copy, Clone, PartialEq, Debug)]
enum Standby {
    Active,
    Dim,
    Off,
}

const DIM_LEVEL: u8 = 40;
/// Seconds of no input before dimming, and before powering the panel down.
/// COUCH_DIM_S and COUCH_OFF_S override them, so the tiers can be watched in
/// seconds rather than minutes.
const DIM_AFTER_S: u64 = 30;
const OFF_AFTER_S: u64 = 120;

/// Bring the panel back, then bring Slint's clock up to date.
///
/// Both the unblank (~430ms of panel re-init) and a backlight write (it goes
/// through the display's command queue and can block for a frame or more)
/// happen inside this call, and an animation started afterwards takes its
/// start time from the tick `update_timers_and_animations` last set - before
/// the block. A key dispatched straight after a wake then started its ring
/// animation already most of the way through: two frames instead of ten,
/// measured. Refreshing the tick here is what makes the first press after a
/// wake glide like any other.
fn wake(screen: &mut Panel) {
    if screen.unblank_if_asleep() {
        println!("couch-gui: standby: panel was asleep, unblanked");
    }
    Panel::set_backlight(255);
    slint::platform::update_timers_and_animations();
}

fn env_secs(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The UI is left on the default affinity deliberately, not pinned off
    // CPU 0. The input EINT interrupts fire only on CPU 0 and freeze it for
    // 46-62ms per press (see README), so it is tempting to pin the UI to the
    // other cores - but the vendor hotplug daemon can take a core offline, and
    // a UI pinned to a core that vanishes freezes for as long as it stays gone
    // (measured: a 4.8s frame). The scheduler already migrates the runnable UI
    // off a core saturated by IRQ time when another is online, which is why the
    // fix is simply to keep a core available (stage2's hotplug floor), never to
    // force the UI onto one.
    let mut screen = Panel::open()?;
    println!("couch-gui: panel {}x{}", screen.width, screen.height);

    let window = CouchPlatform::install(PhysicalSize::new(screen.width, screen.height))
        .map_err(|e| format!("platform: {e:?}"))?;

    Panel::backlight_on();
    screen.claim(BACKGROUND);
    // Whoever resumed the panel before this process started did not
    // re-present it; do it once, so the first frame is seen.
    screen.present();

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

    app.set_area_dots(ModelRc::new(VecModel::from(vec![true; areas.len()])));

    let areas = Rc::new(areas);
    // The area on screen. One index: a page change is applied in one go and
    // the slide is composed from the frame before it and the frame after, so
    // there is no 180ms during which the pager and the page disagree.
    let current = Rc::new(Cell::new(0usize));
    // What the last dispatched event asked for, performed by the loop. One
    // slot: two asks in one iteration - a key and COUCH_SLIDE's timer, say -
    // keep the first, as a second press mid-slide used to be ignored.
    let intent = Rc::new(Cell::new(None::<Intent>));
    let ask = {
        let intent = intent.clone();
        move |what: Intent| {
            if intent.get().is_none() {
                intent.set(Some(what));
            }
        }
    };

    let put_front = {
        let areas = areas.clone();
        move |app: &App, index: usize| {
            let a = &areas[index];
            app.set_area_name(a.name.into());
            app.set_area_index(index as i32);
            app.set_activities(ModelRc::new(VecModel::from(a.activities.clone())));
            app.set_rooms(ModelRc::new(VecModel::from(a.rooms.clone())));
            app.set_scenes(ModelRc::new(VecModel::from(a.scenes.clone())));
            // With the models in place the hub knows where its first room is;
            // it parks focus there and lands the ring, nothing animating.
            app.invoke_page_swapped();
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
    current.set(first);

    // COUCH_FOCUS parks focus on a row - 0 is the activity strip, then one per
    // room, then the scenes row - so any focus position can be photographed
    // without driving the keypad. The one place the host names a row, and it
    // is a test hook: the hub interprets it.
    if let Ok(n) = std::env::var("COUCH_FOCUS").unwrap_or_default().parse::<i32>() {
        app.set_focus_row(n);
        app.invoke_settle_focus();
    }

    {
        let ask = ask.clone();
        app.on_area_step(move |delta| ask(Intent::Area(delta)));
    }

    // OK on the strip or the scenes row opens what is there: straight through
    // when there is one of it, as a list when there are several. A room has
    // no screen behind it yet. The hub says which of the three it was and, for
    // a room, which one; it never hands over a row index. The chooser's
    // content is set here, where the list is known; showing it is the
    // transition, which the loop performs.
    {
        let weak = app.as_weak();
        let areas = areas.clone();
        let current = current.clone();
        let ask = ask.clone();
        app.on_open_strip(move || {
            let Some(app) = weak.upgrade() else { return };
            let a = &areas[current.get()];
            match a.activities.len() {
                0 => return,
                1 => {
                    println!("couch-gui: open activity '{}'", a.activities[0].title);
                    return;
                }
                _ => {}
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
            ask(Intent::OpenChooser);
        });
    }
    {
        let weak = app.as_weak();
        let areas = areas.clone();
        let current = current.clone();
        let ask = ask.clone();
        app.on_open_scenes(move || {
            let Some(app) = weak.upgrade() else { return };
            let a = &areas[current.get()];
            match a.scenes.len() {
                0 => return,
                1 => {
                    println!("couch-gui: run scene '{}'", a.scenes[0].name);
                    return;
                }
                _ => {}
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
            ask(Intent::OpenChooser);
        });
    }
    {
        let areas = areas.clone();
        let current = current.clone();
        app.on_open_room(move |index| {
            if let Some(r) = areas[current.get()].rooms.get(index as usize) {
                println!("couch-gui: open room '{}'", r.name);
            }
        });
    }

    {
        let weak = app.as_weak();
        let ask = ask.clone();
        app.on_chosen(move |index| {
            let Some(app) = weak.upgrade() else { return };
            let title = app
                .get_chooser_items()
                .row_data(index as usize)
                .map(|i| i.title.to_string())
                .unwrap_or_default();
            println!("couch-gui: chose '{title}'");
            ask(Intent::CloseChooser);
        });
    }
    {
        let ask = ask.clone();
        app.on_close_chooser(move || ask(Intent::CloseChooser));
    }
    app.on_back(|| println!("couch-gui: back"));
    app.on_home(|| println!("couch-gui: home"));

    // A page change, performed. The slide is a copy, not a render: the frame
    // on the panel is kept (page A), the change is applied to the UI in one
    // go with the ring hidden and rendered once into RAM (page B), and the
    // panel then shows A pushed out by B a little further each refresh, by
    // memcpy, while the rows that must not move - the status bar, and the
    // pager on an area change - are B throughout. The ring is hidden across
    // the change so B has none; letting it go again afterwards is what starts
    // its 200ms fade in on the next normal frame.
    //
    // Order matters twice over. The snapshot comes before anything changes,
    // because RAM equals the panel only until something is drawn. And the
    // chooser's `changed shown` handler, which puts its list back to the top,
    // runs from update_timers_and_animations - so that runs before B is
    // rendered, or B would show the list where it was last left.
    //
    // Reports what it cost as frames: B's rasterisation and then one per
    // transition frame, so the five-second line counts them with the rest.
    let transition = {
        let areas = areas.clone();
        let current = current.clone();
        let put_front = put_front.clone();
        let report = std::env::var_os("COUCH_REGION").is_some();
        move |screen: &mut Panel, window: &MinimalSoftwareWindow, app: &App, what: Intent|
              -> Option<SlideCost> {
            let chooser = app.get_chooser_shown();
            let status = (0u32, app.get_status_h().round() as u32);
            let dots = (app.get_dots_y().round() as u32, app.get_dots_h().round() as u32);
            let (above, above_and_pager) = ([status], [status, dots]);
            let from;
            let keep: &[(u32, u32)];
            match what {
                Intent::Area(delta) => {
                    let cur = current.get();
                    let next = (cur as i32 + delta).rem_euclid(areas.len() as i32) as usize;
                    if next == cur {
                        return None;
                    }
                    current.set(next);
                    if chooser {
                        // The hub is off screen behind the chooser (COUCH_SLIDE
                        // under COUCH_OPEN): change the page where it is. There
                        // is nothing to see, so nothing to slide.
                        put_front(app, next);
                        return None;
                    }
                    screen.snapshot();
                    app.set_ring_hidden(true);
                    put_front(app, next);
                    from = if delta > 0 { Arrive::FromRight } else { Arrive::FromLeft };
                    keep = &above_and_pager[..];
                }
                Intent::OpenChooser => {
                    if chooser {
                        return None;
                    }
                    screen.snapshot();
                    app.set_ring_hidden(true);
                    app.set_chooser_shown(true);
                    from = Arrive::FromRight;
                    keep = &above[..];
                }
                Intent::CloseChooser => {
                    if !chooser {
                        return None;
                    }
                    screen.snapshot();
                    app.set_ring_hidden(true);
                    app.set_chooser_shown(false);
                    from = Arrive::FromLeft;
                    keep = &above[..];
                }
            }
            slint::platform::update_timers_and_animations();
            let mut cost = SlideCost::default();
            if let Some(us) = screen.render_offscreen(window) {
                if report {
                    println!("couch-gui: slide: page B rendered in {us} us");
                }
                cost.frames += 1;
                cost.work_us += us;
                cost.max_us = us;
                let slid = screen.slide(from, keep, SLIDE);
                cost.frames += slid.frames;
                cost.work_us += slid.work_us;
                cost.wait_us += slid.wait_us;
                cost.max_us = cost.max_us.max(slid.max_us);
            }
            // The ring's fade takes its start time from the animation tick
            // at the moment the flag changes, and that tick only advances in
            // update_timers_and_animations - last called before B, a slide
            // ago. Advance it first, or the fade begins 180ms in and the ring
            // pops rather than fades.
            slint::platform::update_timers_and_animations();
            app.set_ring_hidden(false);
            Some(cost)
        }
    };

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
    // COUCH_KEYBOARD opens the on-screen keyboard a moment in, so it can be
    // driven with the real D-pad and touch on the real renderer.
    let keyboard_at = std::env::var("COUCH_KEYBOARD")
        .is_ok()
        .then(|| now_monotonic_us() + 1_500_000);
    let mut keyboard_opened = false;
    app.on_keyboard_accepted(|t| println!("couch-gui: keyboard accepted '{t}'"));
    app.on_keyboard_cancelled(|| println!("couch-gui: keyboard cancelled"));

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

    // Frame cost is the draw and the copy; the wait for the panel after them
    // is counted apart, because it is slack rather than work.
    let (mut frames, mut render_us, mut frame_max, mut wait_us) = (0u64, 0u64, 0u64, 0u64);

    let dim_after_us = env_secs("COUCH_DIM_S", DIM_AFTER_S) * 1_000_000;
    let off_after_us = env_secs("COUCH_OFF_S", OFF_AFTER_S) * 1_000_000;
    let mut standby = Standby::Active;
    let mut last_input = now_monotonic_us();
    // When to re-assert the backlight after a wake, forced past the LED
    // layer; None when nothing is owed.
    let mut verify_at: Option<u64> = None;
    println!(
        "couch-gui: standby: dim after {}s, off after {}s",
        dim_after_us / 1_000_000,
        off_after_us / 1_000_000
    );
    let (mut in_n, mut in_sum, mut in_max) = (0u64, 0u64, 0u64);
    let mut last_stat = now_monotonic_us();

    loop {
        if let Some(press) = pad.poll() {
            if !press.repeat {
                in_n += 1;
                in_sum += press.latency_us;
                in_max = in_max.max(press.latency_us);
            }
            last_input = now_monotonic_us();
            // A key on a dark panel wakes it and does nothing else - except
            // the microphone key, whose press is the whole intent.
            let swallow = standby == Standby::Off && press.mic != Some(true);
            if standby != Standby::Active {
                println!("couch-gui: standby: wake on key ({:?})", standby);
                wake(&mut screen);
                standby = Standby::Active;
                verify_at = Some(now_monotonic_us() + 1_000_000);
            }
            if swallow {
                continue;
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
            // A touch neither wakes the panel nor counts as input while it is
            // dimmed or dark: a button does. The events are still drained so
            // the first tap after a wake starts from a clean state.
            if standby != Standby::Active {
                continue;
            }
            last_input = now_monotonic_us();
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
        if let Some(at) = keyboard_at {
            if !keyboard_opened && now >= at {
                keyboard_opened = true;
                app.set_keyboard_title("TRY THE KEYBOARD".into());
                app.set_keyboard_placeholder("Type something".into());
                app.invoke_open_keyboard();
            }
        }
        if let Some(at) = open_at {
            if !opened && now >= at {
                opened = true;
                app.invoke_activate();
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
            app.set_wifi_level(system::wifi_level());

            let setup = system::in_setup_mode();
            if last_setup != Some(setup) {
                last_setup = Some(setup);
                app.set_setup_mode(setup);
                if setup {
                    let ssid = system::setup_ssid();
                    app.set_ssid(SharedString::from(ssid.clone()));
                    if let Some(img) = qr::render(&qr::wifi_join_record(&ssid), 296) {
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

            // Standby. Anything a person is looking at or waiting on holds
            // the panel awake and restarts the clock; otherwise it dims, then
            // powers down, on the two idle timers.
            let hold = app.get_pair_shown() || mic.recording() || app.get_setup_mode();
            let idle = now.saturating_sub(last_input);
            // The panel is meant to be showing something in every state but
            // Off. If the driver says it is asleep anyway - it has happened,
            // with nothing in this process asking - bring it back now rather
            // than at the next wake, and re-assert the level the LED node
            // thinks it already has.
            if standby != Standby::Off && screen.unblank_if_asleep() {
                println!("couch-gui: standby: panel found asleep while {:?}, unblanked", standby);
                Panel::force_backlight(if standby == Standby::Dim { DIM_LEVEL } else { 255 });
                slint::platform::update_timers_and_animations();
            }
            if hold {
                last_input = now;
                if standby != Standby::Active {
                    println!("couch-gui: standby: wake to show something ({:?})", standby);
                    wake(&mut screen);
                    standby = Standby::Active;
                    verify_at = Some(now + 1_000_000);
                }
            } else if standby == Standby::Active && idle >= dim_after_us {
                println!("couch-gui: standby: dim after {}s idle", idle / 1_000_000);
                Panel::set_backlight(DIM_LEVEL);
                standby = Standby::Dim;
            } else if standby == Standby::Dim && idle >= off_after_us {
                println!("couch-gui: standby: off after {}s idle", idle / 1_000_000);
                Panel::set_backlight(0);
                screen.blank(true);
                standby = Standby::Off;
            }
        }

        if nav && now >= nav_at {
            // The hub owns the row arithmetic; this only wraps.
            let rows = app.get_row_count();
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

        // Everything above only asked; this is where a page change happens,
        // with no event being dispatched and the draw free to run. It blocks
        // for the slide's duration: keys pressed meanwhile queue in the
        // keypad and are taken one per frame afterwards, so a second Left or
        // Right simply slides again.
        if let Some(what) = intent.take() {
            if let Some(cost) = transition(&mut screen, &window, &app, what) {
                frames += cost.frames;
                render_us += cost.work_us;
                wait_us += cost.wait_us;
                frame_max = frame_max.max(cost.max_us);
            }
        }

        slint::platform::update_timers_and_animations();

        // A drawn frame comes back paced to the panel's refresh, so an
        // animation costs one rasterisation per refresh rather than as many
        // as the CPU can manage. Nothing to draw: a short sleep, and back to
        // polling input.
        // A powered-down panel shows nothing, so nothing is drawn for it:
        // Slint's state keeps advancing (the clock, a PIN arriving) and the
        // first frame after waking catches up. Input is still polled, at a
        // rate a hand cannot notice and the battery can.
        if standby == Standby::Off {
            std::thread::sleep(Duration::from_millis(40));
            continue;
        }

        match screen.render(&window) {
            Some(cost) => {
                render_us += cost.work_us;
                wait_us += cost.wait_us;
                frames += 1;
                frame_max = frame_max.max(cost.work_us);
            }
            None => {
                // Idle, so the ~220ms this costs hitches nothing on screen.
                if verify_at.is_some_and(|t| now_monotonic_us() >= t) && standby == Standby::Active {
                    verify_at = None;
                    if screen.unblank_if_asleep() {
                        println!("couch-gui: standby: panel asleep after wake, unblanked");
                    }
                    Panel::force_backlight(255);
                    slint::platform::update_timers_and_animations();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        if now_monotonic_us() - last_stat > 5_000_000 && frames > 0 {
            println!(
                "couch-gui: {frames} frames, {} us/frame, {frame_max} us worst, {} us paced | input {} us avg, {in_max} us max (n={in_n})",
                render_us / frames,
                wait_us / frames,
                if in_n > 0 { in_sum / in_n } else { 0 }
            );
            last_stat = now_monotonic_us();
            frames = 0;
            render_us = 0;
            frame_max = 0;
            wait_us = 0;
            in_n = 0;
            in_sum = 0;
            in_max = 0;
        }
    }
}
