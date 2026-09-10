//! Home Assistant climate presentation and coalesced commands. All HTTP stays
//! on the worker; navigation invalidates replies and unsent adjustments.
use crate::App;
use couch_ha::{Climate, ClimateCommand};
use slint::{ModelRc, VecModel};
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Target {
    id: String,
    name: String,
    serial: u64,
}
enum Input {
    Open(String, String),
    RoomAdjust(String, String, i32),
    Action(String, i32),
}
enum Operation {
    Read,
    Command(ClimateCommand),
}
enum Answer {
    State(Climate),
    Accepted,
}
fn configured(target: &Target) -> bool {
    crate::config_snapshot::current().is_some_and(|s| {
        s.serial == target.serial
            && s.config.devices().any(|(_, d)| {
                d.kind == couch_model::DeviceKind::Thermostat
                    && matches!(s.config.resolve_integration(&d.integration),
            Some(couch_model::Integration::HomeAssistant{entity_id}) if entity_id == target.id)
            })
    })
}
fn perform(target: &Target, op: Operation) -> Result<Answer, String> {
    if !configured(target) {
        return Err("Thermostat configuration changed. Reopen the device.".into());
    }
    let (client, id) = crate::connections::ha(&target.id)?;
    match op {
        Operation::Read => client
            .climate(&id)
            .map(Answer::State)
            .map_err(|e| e.to_string()),
        Operation::Command(command) => client
            .climate_command(&id, command)
            .map(|()| Answer::Accepted)
            .map_err(|e| e.to_string()),
    }
}

pub struct Controller {
    input: Rc<RefCell<VecDeque<Input>>>,
    tx: mpsc::SyncSender<(u64, Target, Operation)>,
    rx: mpsc::Receiver<(u64, Result<Answer, String>)>,
    generation: u64,
    active_generation: Arc<AtomicU64>,
    in_flight: Option<ClimateCommand>,
    mode_wait: bool,
    mode_expected: Option<String>,
    mode_deadline: Instant,
    last_render: Option<(Option<Climate>, String, String, bool, bool)>,
    target: Option<Target>,
    state: Option<Climate>,
    state_at: Instant,
    busy: bool,
    desired: Option<ClimateCommand>,
    queued_delta: i32,
    due: Instant,
    next_read: Instant,
    error: String,
    notice: Option<String>,
    feedback_until: Option<Instant>,
    feedback_room: Option<String>,
}
impl Controller {
    pub fn new(app: &App) -> Self {
        let input = Rc::new(RefCell::new(VecDeque::new()));
        let q = input.clone();
        app.on_open_thermostat(move |id, name| {
            q.borrow_mut()
                .push_back(Input::Open(id.to_string(), name.to_string()))
        });
        let q = input.clone();
        app.on_thermostat_room_adjust(move |id, name, delta| {
            q.borrow_mut()
                .push_back(Input::RoomAdjust(id.to_string(), name.to_string(), delta))
        });
        let q = input.clone();
        app.on_thermostat_action(move |name, index| {
            q.borrow_mut()
                .push_back(Input::Action(name.to_string(), index))
        });
        let (tx, requests) = mpsc::sync_channel::<(u64, Target, Operation)>(1);
        let (events, rx) = mpsc::channel();
        let active_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = active_generation.clone();
        std::thread::spawn(move || {
            while let Ok((generation, target, op)) = requests.recv() {
                if worker_generation.load(Ordering::Acquire) != generation {
                    continue;
                }
                if events.send((generation, perform(&target, op))).is_err() {
                    break;
                }
            }
        });
        Self {
            input,
            tx,
            rx,
            generation: 0,
            active_generation,
            in_flight: None,
            mode_wait: false,
            mode_expected: None,
            mode_deadline: Instant::now(),
            last_render: None,
            target: None,
            state: None,
            state_at: Instant::now(),
            busy: false,
            desired: None,
            queued_delta: 0,
            due: Instant::now(),
            next_read: Instant::now(),
            error: String::new(),
            notice: None,
            feedback_until: None,
            feedback_room: None,
        }
    }
    pub fn navigation_pending(&self) -> bool {
        self.input.borrow().iter().any(|v| {
            matches!(v, Input::Open(..))
                || matches!(v,Input::Action(a,_) if a=="close" || a=="home")
        })
    }
    fn invalidate(&mut self) {
        self.generation += 1;
        self.active_generation
            .store(self.generation, Ordering::Release);
        if self.desired.is_some() || self.in_flight.is_some() || self.mode_wait {
            self.state = None;
        }
        self.in_flight = None;
        self.mode_wait = false;
        self.mode_expected = None;
    }
    fn select(&mut self, id: String, name: String) {
        let serial = crate::config_snapshot::current().map_or(0, |s| s.serial);
        let same = self
            .target
            .as_ref()
            .is_some_and(|t| t.id == id && t.serial == serial);
        if !same || self.state_at.elapsed() > Duration::from_secs(30) {
            self.state = None;
        }
        self.invalidate();
        self.target = Some(Target { id, name, serial });
        self.busy = false;
        self.desired = None;
        self.queued_delta = 0;
        self.error.clear();
        self.next_read = Instant::now();
    }
    fn close(&mut self, app: &App, home: bool) {
        self.invalidate();
        self.busy = false;
        self.desired = None;
        self.queued_delta = 0;
        app.set_thermostat_modes_shown(false);
        app.set_thermostat_shown(false);
        if home {
            app.invoke_light_back();
        } else if app.get_light_shown() {
            app.invoke_focus_light();
        } else {
            app.invoke_focus_home();
        }
    }
    fn adjust(&mut self, app: &App, delta: i32) {
        if self.mode_wait {
            self.error = "Waiting for the thermostat mode to update.".into();
            return;
        }
        let Some(state) = self.state.as_ref() else {
            self.queued_delta = (self.queued_delta + delta).clamp(-20, 20);
            self.next_read = Instant::now();
            return;
        };
        match state.adjusted_target(delta, self.desired.as_ref()) {
            Ok(command) => {
                apply_target(self.state.as_mut().unwrap(), &command);
                self.desired = Some(command);
                self.due = Instant::now() + Duration::from_millis(250);
                self.error.clear();
                if !app.get_thermostat_shown() {
                    app.set_thermostat_feedback_name(
                        self.target.as_ref().unwrap().name.as_str().into(),
                    );
                    app.set_thermostat_feedback_value(
                        target_text(self.state.as_ref().unwrap()).into(),
                    );
                    app.set_feedback_enabled(true);
                    app.set_brightness_shown(false);
                    app.set_scene_feedback_shown(false);
                    app.set_thermostat_feedback_shown(true);
                    self.feedback_until = Some(Instant::now() + Duration::from_secs(1));
                }
            }
            Err(e) => self.fail(app, e.to_string()),
        }
    }
    fn fail(&mut self, app: &App, error: String) {
        app.set_thermostat_feedback_shown(false);
        self.feedback_until = None;
        self.error = error.clone();
        self.desired = None;
        self.queued_delta = 0;
        if !app.get_thermostat_shown() {
            self.notice = Some(error);
        }
    }
    fn send(&mut self, op: Operation) -> bool {
        let Some(target) = self.target.clone() else {
            return false;
        };
        if self.tx.try_send((self.generation, target, op)).is_ok() {
            self.busy = true;
            true
        } else {
            false
        }
    }
    fn render(&mut self, app: &App) {
        if !app.get_thermostat_shown() {
            self.last_render = None;
            return;
        }
        let key = (
            self.state.clone(),
            self.error.clone(),
            self.target
                .as_ref()
                .map(|t| t.name.clone())
                .unwrap_or_default(),
            self.desired.is_some() || self.in_flight.is_some(),
            self.mode_wait,
        );
        if self.last_render.as_ref() == Some(&key) {
            return;
        }
        self.last_render = Some(key);
        if let Some(t) = &self.target {
            app.set_thermostat_title(t.name.as_str().into());
        }
        let pending = self.desired.is_some() || self.in_flight.is_some() || self.mode_wait;
        app.set_thermostat_pending(pending);
        if let Some(s) = &self.state {
            app.set_thermostat_current(
                temperature(s.current_temperature, &s.temperature_unit).into(),
            );
            app.set_thermostat_target(target_text(s).into());
            app.set_thermostat_range(range(s));
            app.set_thermostat_adjustable(!self.mode_wait && s.adjusted_target(0, None).is_ok());
            app.set_thermostat_status(if !s.available {
                "Unavailable".into()
            } else {
                label(
                    s.hvac_action
                        .as_deref()
                        .or(s.hvac_mode.as_deref())
                        .unwrap_or("unknown"),
                )
                .into()
            });
            app.set_thermostat_mode(label(s.hvac_mode.as_deref().unwrap_or("unknown")).into());
            app.set_thermostat_modes(ModelRc::new(VecModel::from(if s.available {
                s.hvac_modes.iter().map(|m| label(m).into()).collect()
            } else {
                Vec::new()
            })));
        } else {
            app.set_thermostat_current("—".into());
            app.set_thermostat_target("—".into());
            app.set_thermostat_status("Connecting…".into());
            app.set_thermostat_mode("Checking mode…".into());
            app.set_thermostat_modes(ModelRc::default());
            app.set_thermostat_range(false);
            app.set_thermostat_adjustable(false);
        }
        app.set_thermostat_detail(if !self.error.is_empty() {
            self.error.as_str().into()
        } else if self
            .state
            .as_ref()
            .is_some_and(|s| s.hvac_mode.as_deref() == Some("off"))
        {
            "Choose a mode to adjust temperature.".into()
        } else if self
            .state
            .as_ref()
            .is_some_and(|s| s.adjusted_target(0, None).is_err())
        {
            "Target adjustment is unavailable.\nOK chooses the thermostat mode.".into()
        } else if self.state.as_ref().is_some_and(range) {
            "Volume adjusts both setpoints.\nOK chooses the thermostat mode.".into()
        } else {
            "Volume adjusts the target.\nOK chooses the thermostat mode.".into()
        });
    }
    pub fn poll(&mut self, app: &App) -> Option<String> {
        if self.feedback_room.as_ref().is_some_and(|room| {
            room != app.get_light_room_id().as_str()
                || !app.get_light_shown()
                || app.get_thermostat_shown()
                || app.get_tv_shown()
                || app.get_player_shown()
                || app.get_chooser_shown()
                || app.get_settings_shown()
                || app.get_keyboard_shown()
                || app.get_pair_shown()
                || app.get_wifi_setup_shown()
        }) {
            self.invalidate();
            self.desired = None;
            self.queued_delta = 0;
            self.busy = false;
            self.target = None;
            self.feedback_room = None;
            self.feedback_until = None;
            self.notice = None;
            app.set_thermostat_feedback_shown(false);
        }
        if self
            .feedback_until
            .is_some_and(|until| Instant::now() >= until)
        {
            self.feedback_until = None;
            app.set_thermostat_feedback_shown(false);
        }
        loop {
            let Some(input) = self.input.borrow_mut().pop_front() else {
                break;
            };
            match input {
                Input::Open(id, name) => {
                    self.feedback_room = None;
                    self.feedback_until = None;
                    app.set_thermostat_feedback_shown(false);
                    self.select(id, name);
                    app.set_active_activity("".into());
                    app.set_thermostat_modes_shown(false);
                    app.set_thermostat_shown(true);
                    app.invoke_focus_thermostat();
                }
                Input::RoomAdjust(id, name, delta) => {
                    self.feedback_room = Some(app.get_light_room_id().to_string());
                    let serial = crate::config_snapshot::current().map_or(0, |s| s.serial);
                    if !self
                        .target
                        .as_ref()
                        .is_some_and(|t| t.id == id && t.serial == serial)
                    {
                        self.select(id, name);
                    }
                    if self.state_at.elapsed() > Duration::from_secs(5)
                        && !self.busy
                        && self.desired.is_none()
                    {
                        self.state = None;
                    }
                    self.adjust(app, delta.signum());
                }
                Input::Action(action, index) => match action.as_str() {
                    "close" | "home" => self.close(app, action == "home"),
                    "dismiss" => app.set_thermostat_modes_shown(false),
                    "adjust" => self.adjust(app, index.signum()),
                    "modes" => {
                        if self
                            .state
                            .as_ref()
                            .is_some_and(|s| s.available && !s.hvac_modes.is_empty())
                        {
                            app.set_thermostat_modes_shown(true);
                        }
                    }
                    "mode" => {
                        if let Some(mode) = self
                            .state
                            .as_ref()
                            .filter(|s| s.available)
                            .and_then(|s| s.hvac_modes.get(index as usize))
                            .cloned()
                        {
                            // Finish an in-flight temperature change before changing mode;
                            // replace any unsent temperature change with explicit mode intent.
                            self.mode_wait = true;
                            self.mode_expected = Some(mode.clone());
                            self.mode_deadline = Instant::now() + Duration::from_secs(15);
                            self.desired = Some(ClimateCommand::HvacMode(mode));
                            self.due = Instant::now();
                            self.error.clear();
                            app.set_thermostat_modes_shown(false);
                        }
                    }
                    _ => {}
                },
            }
        }
        if self.target.as_ref().is_some_and(|t| !configured(t)) {
            self.invalidate();
            self.busy = false;
            self.state = None;
            self.target = None;
            self.fail(
                app,
                "Thermostat configuration changed. Return to the room and reopen it.".into(),
            );
        }
        while let Ok((generation, result)) = self.rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.busy = false;
            self.in_flight = None;
            match result {
                Ok(Answer::State(state)) => {
                    // A read started before a key press must not erase the new target.
                    let mut state = state;
                    if let Some(cmd) = &self.desired {
                        apply_target(&mut state, cmd);
                    }
                    if self.desired.is_none() && self.mode_wait {
                        if state.hvac_mode == self.mode_expected {
                            self.mode_wait = false;
                            self.mode_expected = None;
                            self.error.clear();
                        } else if Instant::now() >= self.mode_deadline {
                            self.mode_wait = false;
                            self.mode_expected = None;
                            self.error = "Mode change was not confirmed. Try again.".into();
                        }
                    }
                    self.state = Some(state);
                    self.state_at = Instant::now();
                    self.next_read =
                        Instant::now() + Duration::from_secs(if self.mode_wait { 1 } else { 5 });
                    let delta = std::mem::take(&mut self.queued_delta);
                    if delta != 0 {
                        self.adjust(app, delta);
                    }
                }
                Ok(Answer::Accepted) => {
                    self.next_read = Instant::now() + Duration::from_millis(1500);
                }
                Err(error) => {
                    self.mode_wait = false;
                    self.state = None;
                    self.next_read = Instant::now() + Duration::from_secs(5);
                    self.fail(app, error);
                }
            }
        }
        if !self.busy && self.target.is_some() {
            if let Some(command) = self.desired.clone().filter(|_| Instant::now() >= self.due) {
                if self.send(Operation::Command(command.clone())) {
                    self.in_flight = Some(command);
                    self.desired = None;
                }
            } else if self.desired.is_none()
                && (app.get_thermostat_shown() || self.queued_delta != 0)
                && Instant::now() >= self.next_read
            {
                self.send(Operation::Read);
            }
        }
        self.render(app);
        self.notice.take()
    }
}
fn label(s: &str) -> String {
    match s {
        "heat_cool" => "Heat / cool".into(),
        "fan_only" => "Fan only".into(),
        "unknown" => "Unknown".into(),
        "dry" => "Dry".into(),
        _ => {
            let mut c = s.chars();
            c.next()
                .map(|a| a.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        }
    }
}
fn temperature(value: Option<f64>, unit: &str) -> String {
    value
        .filter(|v| v.is_finite())
        .map(|v| {
            format!(
                "{}{}",
                format!("{v:.3}")
                    .trim_end_matches('0')
                    .trim_end_matches('.'),
                unit
            )
        })
        .unwrap_or_else(|| "—".into())
}
fn range(s: &Climate) -> bool {
    s.hvac_mode.as_deref() == Some("heat_cool")
        || (!s.supports_target_temperature && s.supports_target_range)
}
fn target_text(s: &Climate) -> String {
    if !s.available {
        return "—".into();
    }
    if range(s) {
        format!(
            "{} – {}",
            temperature(s.target_temperature_low, &s.temperature_unit),
            temperature(s.target_temperature_high, &s.temperature_unit)
        )
    } else {
        temperature(s.target_temperature, &s.temperature_unit)
    }
}
fn apply_target(state: &mut Climate, command: &ClimateCommand) {
    match command {
        ClimateCommand::Temperature(t) => state.target_temperature = Some(*t),
        ClimateCommand::TemperatureRange { low, high } => {
            state.target_temperature_low = Some(*low);
            state.target_temperature_high = Some(*high);
        }
        ClimateCommand::HvacMode(_) => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temperature_text_preserves_units_and_unknown() {
        assert_eq!(temperature(Some(21.5), "°C"), "21.5°C");
        assert_eq!(temperature(Some(21.25), "°C"), "21.25°C");
        assert_eq!(temperature(Some(72.), "°F"), "72°F");
        assert_eq!(temperature(None, "°C"), "—");
        assert_eq!(temperature(Some(f64::NAN), "°C"), "—");
    }
    #[test]
    fn range_presentation_and_pending_commands_preserve_independent_limits() {
        let mut s=Climate::from_state(&serde_json::json!({"entity_id":"climate.test","state":"heat_cool","attributes":{"supported_features":3,"min_temp":10,"max_temp":35,"target_temp_low":20,"target_temp_high":24}}),"°C").unwrap();
        assert_eq!(target_text(&s), "20°C – 24°C");
        let cmd = s.adjusted_target(1, None).unwrap();
        apply_target(&mut s, &cmd);
        assert_eq!(target_text(&s), "20.5°C – 24.5°C");
        apply_target(&mut s, &ClimateCommand::HvacMode("off".into()));
        assert_eq!(
            s.hvac_mode.as_deref(),
            Some("heat_cool"),
            "mode remains confirmed until HA reports it"
        );
    }
    #[test]
    fn thermostat_keys_and_pending_navigation_are_safe() {
        if std::env::var_os("COUCH_TEST_THERMOSTAT").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "thermostat::tests::thermostat_keys_and_pending_navigation_are_safe",
                ])
                .env("COUCH_TEST_THERMOSTAT", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use slint::{
            platform::{Key, WindowEvent},
            ComponentHandle,
        };
        let window =
            crate::panel::CouchPlatform::install(slint::PhysicalSize::new(480, 800)).unwrap();
        let app = App::new().unwrap();
        let mut controller = Controller::new(&app);
        app.set_thermostat_shown(true);
        app.show().unwrap();
        app.invoke_focus_thermostat();
        for key in [Key::F23, Key::F24, Key::Return, Key::Escape] {
            window.dispatch_event(WindowEvent::KeyPressed {
                text: char::from(key).to_string().into(),
            });
        }
        let events: Vec<_> = controller
            .input
            .borrow_mut()
            .drain(..)
            .map(|i| match i {
                Input::Action(a, d) => (a, d),
                _ => panic!("unexpected input"),
            })
            .collect();
        assert_eq!(
            events,
            vec![
                ("adjust".into(), 1),
                ("adjust".into(), -1),
                ("modes".into(), 0),
                ("close".into(), 0)
            ]
        );
        controller.state = Climate::from_state(
            &serde_json::json!({"entity_id":"climate.test","state":"heat","attributes":{"supported_features":1,"temperature":21,"min_temp":10,"max_temp":35}}),
            "°C",
        );
        controller.mode_wait = true;
        controller.desired = Some(ClimateCommand::HvacMode("cool".into()));
        controller.adjust(&app, 1);
        assert_eq!(
            controller.desired,
            Some(ClimateCommand::HvacMode("cool".into())),
            "volume must not replace a mode command"
        );
        controller.mode_wait = false;
        controller.desired = None;
        controller.adjust(&app, 1);
        controller.adjust(&app, 1);
        assert_eq!(
            controller.desired,
            Some(ClimateCommand::Temperature(22.)),
            "repeat presses coalesce from latest target"
        );
        controller.close(&app, false);
        assert!(controller.desired.is_none());
        assert!(
            controller.state.is_none(),
            "discarded optimistic target cannot appear on reentry"
        );
        assert_eq!(
            controller.active_generation.load(Ordering::Acquire),
            controller.generation,
            "worker sees cancellation before next operation"
        );
        assert!(!app.get_thermostat_shown());
        controller.target = Some(Target {
            id: "test/climate.test".into(),
            name: "Living room".into(),
            serial: 0,
        });
        controller.state = Climate::from_state(
            &serde_json::json!({"entity_id":"climate.test","state":"heat","attributes":{"supported_features":1,"temperature":21,"min_temp":10,"max_temp":35}}),
            "°C",
        );
        app.set_light_shown(true);
        app.set_light_room_id("living".into());
        controller.feedback_room = Some("living".into());
        controller.adjust(&app, 1);
        assert!(app.get_thermostat_feedback_shown());
        assert_eq!(app.get_thermostat_feedback_value(), "21.5°C");
        assert!(
            controller.notice.is_none(),
            "target feedback uses shared card, not small error toast"
        );
        app.set_light_room_id("bedroom".into());
        controller.poll(&app);
        assert!(!app.get_thermostat_feedback_shown());
        assert!(controller.desired.is_none());
    }
}
