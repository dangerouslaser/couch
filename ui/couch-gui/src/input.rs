//! Physical input policy: wake consumption, touch arbitration and key holds.
use crate::{keypad::now_monotonic_us, panel::Panel, touch};
/// How much of the panel is on.
///
/// Three levels, two timers. Dimmed, the screen is still readable and the
/// first key acts as it always would; off, the panel is powered down (LCM,
/// backlight PWM and the touch controller all suspended by the driver) and
/// the first key only wakes it - a dark remote should not change the house
/// because someone found the wrong button in the dark. The microphone key is
/// the exception: holding it in the dark means "talk", so it wakes and records.
/// A first tap wakes from dim without selecting a target. Full powerdown
/// suspends the touch controller and still requires a button to wake.
///
/// A pairing PIN on screen, a recording in progress, or first-run setup hold
/// the panel awake: each is something a person is looking at or waiting on.
///
/// A second after every wake the backlight is written once more, forced past
/// the LED layer's deduplication - see `Panel::set_backlight` for the dropped
/// write that left the panel stuck dim while the LED node said 255.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Standby {
    Active,
    Dim,
    Off,
}

#[derive(Debug, PartialEq)]
pub enum TouchDisposition {
    Ignore,
    Wake,
    Dispatch,
}

pub fn touch_disposition(
    state: Standby,
    event: &touch::Event,
    swallow: &mut bool,
) -> TouchDisposition {
    if state == Standby::Off {
        return TouchDisposition::Ignore;
    }
    if *swallow {
        if matches!(event, touch::Event::Released { .. }) {
            *swallow = false;
        }
        return TouchDisposition::Ignore;
    }
    if state == Standby::Dim {
        if matches!(event, touch::Event::Pressed { .. }) {
            *swallow = true;
            return TouchDisposition::Wake;
        }
        return TouchDisposition::Ignore;
    }
    TouchDisposition::Dispatch
}

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
pub fn wake(screen: &mut Panel, level: u8) {
    let started = now_monotonic_us();
    if screen.unblank_if_asleep() {
        println!("couch-gui: standby: panel was asleep, unblanked");
    }
    let presented = now_monotonic_us();
    Panel::set_backlight(level);
    println!(
        "couch-gui: wake: panel/present={}ms, backlight={}ms",
        (presented - started) / 1000,
        (now_monotonic_us() - presented) / 1000
    );
    slint::platform::update_timers_and_animations();
}

#[derive(Default)]
pub struct Physical {
    mic_down_at: u64,
    menu_down_at: Option<u64>,
    latched: bool,
}
#[derive(Debug, PartialEq)]
pub enum MicAction {
    None,
    Start,
    Stop,
}
impl Physical {
    pub fn menu_edge(&mut self, edge: Option<bool>, now: u64) {
        match edge {
            Some(true) => self.menu_down_at = Some(now),
            Some(false) => self.menu_down_at = None,
            None => {}
        }
    }
    pub fn settings_hold_due(&mut self, now: u64, on_home: bool) -> bool {
        if on_home
            && self
                .menu_down_at
                .is_some_and(|t| now.saturating_sub(t) >= 500_000)
        {
            self.menu_down_at = None;
            true
        } else {
            false
        }
    }
    pub fn microphone(&mut self, edge: Option<bool>, now: u64, recording: bool) -> MicAction {
        match edge {
            Some(true) => {
                if self.latched {
                    self.latched = false;
                    MicAction::Stop
                } else {
                    self.mic_down_at = now;
                    MicAction::Start
                }
            }
            Some(false) if recording => {
                if now.saturating_sub(self.mic_down_at) < 600_000 {
                    self.latched = true;
                    MicAction::None
                } else {
                    MicAction::Stop
                }
            }
            _ => MicAction::None,
        }
    }
    pub fn sync_microphone(&mut self, recording: bool) {
        if !recording {
            self.latched = false;
        }
    }
    pub fn latched(&self) -> bool {
        self.latched
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn microphone_tap_latches_hold_releases_and_capture_limit_clears_latch() {
        let mut p = Physical::default();
        assert_eq!(p.microphone(Some(true), 100, false), MicAction::Start);
        assert_eq!(p.microphone(Some(false), 100_000, true), MicAction::None);
        assert!(p.latched());
        assert_eq!(p.microphone(Some(true), 200_000, true), MicAction::Stop);
        assert!(!p.latched());
        assert_eq!(p.microphone(Some(true), 300_000, false), MicAction::Start);
        assert_eq!(p.microphone(Some(false), 1_000_000, true), MicAction::Stop);
        p.microphone(Some(true), 2_000_000, false);
        p.microphone(Some(false), 2_100_000, true);
        p.sync_microphone(false);
        assert!(!p.latched());
    }
    #[test]
    fn menu_hold_only_opens_settings_on_home_and_only_once() {
        let mut p = Physical::default();
        p.menu_edge(Some(true), 1);
        assert!(!p.settings_hold_due(600_000, false));
        assert!(p.settings_hold_due(600_000, true));
        assert!(!p.settings_hold_due(700_000, true));
        p.menu_edge(Some(true), 1_000_000);
        p.menu_edge(Some(false), 1_200_000);
        assert!(!p.settings_hold_due(2_000_000, true));
    }
}

/// Reserve a Back hold while a device owns the physical keys. Keep swallowing
/// its release after navigation so it cannot activate the previous screen.
#[derive(Default)]
pub struct BackHold {
    context: String,
    captured: bool,
    pending: Option<(crate::keypad::Press, u64)>,
}
pub enum BackAction {
    Pass(crate::keypad::Press),
    Consume,
    Exit,
}
impl BackHold {
    pub fn context(&mut self, context: String) {
        if self.context != context {
            self.context = context;
            self.pending = None;
        }
    }
    pub fn poll(&mut self, now: u64) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_sub(*at) >= 600_000)
        {
            self.pending = None;
            true
        } else {
            false
        }
    }
    pub fn handle(&mut self, press: crate::keypad::Press, now: u64, awake: bool) -> BackAction {
        if press.code != 158 {
            return BackAction::Pass(press);
        }
        if press.released && self.captured {
            self.captured = false;
            return match self.pending.take() {
                Some((down, at)) if now.saturating_sub(at) < 600_000 => BackAction::Pass(down),
                Some(_) => BackAction::Exit,
                None => BackAction::Consume,
            };
        }
        if self.captured {
            return BackAction::Consume;
        }
        if self.context.is_empty() || !awake || press.released || press.repeat {
            return BackAction::Pass(press);
        }
        self.captured = true;
        self.pending = Some((press, now));
        BackAction::Consume
    }
}

#[cfg(test)]
mod back_tests {
    use super::*;
    fn press(released: bool) -> crate::keypad::Press {
        crate::keypad::Press {
            code: 158,
            released,
            key: None,
            mic: None,
            menu: None,
            latency_us: 0,
            repeat: false,
        }
    }
    #[test]
    fn short_back_is_delivered_on_release_and_hold_exits_only_once() {
        let mut hold = BackHold::default();
        hold.context("kodi".into());
        assert!(matches!(
            hold.handle(press(false), 0, true),
            BackAction::Consume
        ));
        assert!(
            matches!(hold.handle(press(true), 100_000, true), BackAction::Pass(p) if !p.released)
        );
        assert!(matches!(
            hold.handle(press(false), 200_000, true),
            BackAction::Consume
        ));
        assert!(!hold.poll(799_999));
        assert!(hold.poll(800_000));
        assert!(!hold.poll(900_000));
        hold.context(String::new());
        assert!(matches!(
            hold.handle(press(true), 1_000_000, true),
            BackAction::Consume
        ));
    }
    #[test]
    fn navigation_cancels_hold_and_dark_screen_does_not_arm_it() {
        let mut hold = BackHold::default();
        hold.context("tv".into());
        assert!(matches!(
            hold.handle(press(false), 0, false),
            BackAction::Pass(_)
        ));
        assert!(!hold.poll(700_000));
        hold.handle(press(false), 1_000_000, true);
        hold.context("another-device".into());
        assert!(!hold.poll(1_700_000));
        assert!(matches!(
            hold.handle(press(true), 1_800_000, true),
            BackAction::Consume
        ));
    }
    #[test]
    fn release_after_slow_frame_still_exits_without_short_back() {
        let mut hold = BackHold::default();
        hold.context("kodi".into());
        hold.handle(press(false), 0, true);
        assert!(matches!(
            hold.handle(press(true), 650_000, true),
            BackAction::Exit
        ));
        assert!(!hold.poll(700_000));
    }
}
