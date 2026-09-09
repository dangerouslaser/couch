//! Activity overrides: physical timing on the UI thread, device I/O on a worker.
use crate::{connections, keypad::Press, App};
use couch_model::{
    buttons::{Binding, Button, Gesture},
    Action, Config, Integration,
};
use serde_json::json;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
const HOLD: Duration = Duration::from_millis(600);
struct Pending {
    down: Press,
    at: Instant,
    fired: bool,
}
struct Request {
    generation: u64,
    at: Instant,
    config: Arc<Config>,
    action: Action,
}
pub struct Controller {
    context: String,
    config: Arc<Config>,
    bindings: Vec<Binding>,
    pending: HashMap<u16, Pending>,
    replay: VecDeque<Press>,
    generation: Arc<AtomicU64>,
    tx: mpsc::SyncSender<Request>,
    rx: mpsc::Receiver<(u64, String)>,
}
impl Controller {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel::<Request>(8);
        let (reply, out) = mpsc::sync_channel(8);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        std::thread::spawn(move || {
            let mut denon = HashMap::new();
            let mut tv = HashMap::new();
            while let Ok(r) = rx.recv() {
                if r.generation != current.load(Ordering::SeqCst)
                    || r.at.elapsed() > Duration::from_millis(750)
                {
                    continue;
                }
                if let Err(error) = execute(&r.config, &r.action, &mut denon, &mut tv) {
                    let _ = reply.try_send((r.generation, error));
                }
            }
        });
        Self {
            context: String::new(),
            config: Arc::new(Config::default()),
            bindings: vec![],
            pending: HashMap::new(),
            replay: VecDeque::new(),
            generation,
            tx,
            rx: out,
        }
    }
    fn binding(&self, button: Button, gesture: Gesture) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|b| b.button == button && b.gesture == gesture)
    }
    fn fire(&self, button: Button, gesture: Gesture, repeat: bool) -> bool {
        let Some(binding) = self.binding(button, gesture) else {
            return false;
        };
        if let Some(action) = &binding.action {
            if !repeat || couch_model::buttons::repeatable(&action.command) {
                let _ = self.tx.try_send(Request {
                    generation: self.generation.load(Ordering::SeqCst),
                    at: Instant::now(),
                    config: self.config.clone(),
                    action: action.clone(),
                });
            }
        }
        true
    }
    pub fn next_replay(&mut self) -> Option<Press> {
        self.replay.pop_front()
    }
    pub fn handle(&mut self, app: &App, press: &Press) -> bool {
        if self.context.is_empty()
            || app.get_pair_shown()
            || app.get_settings_shown()
            || app.get_keyboard_shown()
        {
            return false;
        }
        self.handle_press(press)
    }
    fn handle_press(&mut self, press: &Press) -> bool {
        let Some(button) = Button::from_evdev(press.code) else {
            return false;
        };
        if press.released {
            if let Some(pending) = self.pending.remove(&press.code) {
                if !pending.fired && pending.at.elapsed() >= HOLD {
                    self.fire(button, Gesture::Long, false);
                    return true;
                }
                if !pending.fired && !self.fire(button, Gesture::Short, false) {
                    self.replay.push_back(pending.down);
                    self.replay.push_back(press.clone());
                }
                return true;
            }
            return self.binding(button, Gesture::Short).is_some();
        }
        if self.binding(button, Gesture::Long).is_some() {
            if !press.repeat {
                self.pending.entry(press.code).or_insert_with(|| Pending {
                    down: press.clone(),
                    at: Instant::now(),
                    fired: false,
                });
            }
            return true;
        }
        self.fire(button, Gesture::Short, press.repeat)
    }
    pub fn poll(&mut self, app: &App) -> Option<String> {
        let context = if app.get_player_shown() || app.get_tv_shown() {
            app.get_active_activity().to_string()
        } else {
            String::new()
        };
        if context != self.context {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.pending.clear();
            self.replay.clear();
            self.bindings.clear();
            if let Some(config) = connections::config().filter(|c| c.validate().is_ok()) {
                self.bindings = config
                    .activities
                    .iter()
                    .find(|a| a.id.as_str() == context)
                    .map(|a| a.buttons.clone())
                    .unwrap_or_default();
                self.config = Arc::new(config);
            }
            self.context = context;
        }
        let due: Vec<_> = self
            .pending
            .iter_mut()
            .filter_map(|(&code, p)| {
                if !p.fired && p.at.elapsed() >= HOLD {
                    p.fired = true;
                    Button::from_evdev(code)
                } else {
                    None
                }
            })
            .collect();
        for button in due {
            self.fire(button, Gesture::Long, false);
        }
        let generation = self.generation.load(Ordering::SeqCst);
        self.rx
            .try_iter()
            .filter(|(g, _)| *g == generation)
            .map(|(_, e)| e)
            .last()
    }
}
fn execute(
    config: &Config,
    action: &Action,
    denon: &mut HashMap<String, couch_denon::Client>,
    tv: &mut HashMap<String, couch_webos::Client>,
) -> Result<(), String> {
    let device = config
        .devices()
        .find(|(_, d)| d.id == action.device)
        .map(|(_, d)| d)
        .ok_or("Mapped device was removed")?;
    let integration = config
        .resolve_integration(&device.integration)
        .ok_or("Mapped connection was removed")?;
    if !couch_model::buttons::functions(&integration)
        .iter()
        .any(|f| f.0 == action.command)
    {
        return Err("Unsupported button function".into());
    }
    let connection = match &device.integration {
        Integration::Connection { connection_id, .. } => connection_id.as_str(),
        _ => "",
    };
    let command = action.command.as_str();
    match integration {
        Integration::Denon { host, port } => {
            let key = format!("{host}:{port}");
            if !denon.contains_key(&key) {
                denon.insert(
                    key.clone(),
                    couch_denon::Client::connect(&couch_denon::Settings { host, port })
                        .map_err(|e| e.to_string())?,
                );
            }
            let c = denon.get_mut(&key).unwrap();
            let result = (|| {
                use couch_denon::Command as C;
                let cmd = match command {
                    "power-on" => C::Power(true),
                    "power-off" => C::Power(false),
                    "volume-up" => C::VolumeUp,
                    "volume-down" => C::VolumeDown,
                    "mute-on" => C::Mute(true),
                    "mute-off" => C::Mute(false),
                    "mute" => C::Mute(!c.status()?.muted.ok_or(couch_denon::Error::Protocol)?),
                    _ => return Err(couch_denon::Error::Invalid),
                };
                c.command(cmd).map(|_| ())
            })();
            if result.is_err() {
                denon.remove(&key);
            }
            result.map_err(|e| e.to_string())
        }
        Integration::Kodi { host, port } => {
            let c = couch_kodi::settings::Settings::load(&connections::file(connection, "kodi"))
                .ok()
                .filter(|s| s.host == host && s.http_control)
                .map(|s| s.client())
                .unwrap_or_else(|| couch_kodi::Kodi::tcp(&host, port))
                .with_timeout(Duration::from_secs(2));
            let result = match command {
                "up" | "down" | "left" | "right" | "ok" | "back" | "home" | "menu" => {
                    let method = match command {
                        "up" => "Input.Up",
                        "down" => "Input.Down",
                        "left" => "Input.Left",
                        "right" => "Input.Right",
                        "ok" => "Input.Select",
                        "back" => "Input.Back",
                        "home" => "Input.Home",
                        _ => "Input.ContextMenu",
                    };
                    c.call(method, json!({})).map(|_| ())
                }
                "volume-up" | "volume-down" => c
                    .call(
                        "Application.SetVolume",
                        json!({"volume":if command=="volume-up"{"increment"}else{"decrement"}}),
                    )
                    .map(|_| ()),
                "mute" => c
                    .call("Application.SetMute", json!({"mute":"toggle"}))
                    .map(|_| ()),
                _ => {
                    let p = c
                        .playback()
                        .map_err(|e| e.to_string())?
                        .ok_or("Kodi has no active playback")?;
                    let (method, params) = match command {
                        "play-pause" => ("Player.PlayPause", json!({})),
                        "stop" => ("Player.Stop", json!({})),
                        "next" => ("Player.GoTo", json!({"to":"next"})),
                        _ => ("Player.GoTo", json!({"to":"previous"})),
                    };
                    c.player_command(p.player, method, params).map(|_| ())
                }
            };
            result.map_err(|e| e.to_string())
        }
        Integration::WebOs => {
            if command == "power-on" {
                let path = connections::file(connection, "webos");
                let settings = couch_webos::Settings::load(&path).map_err(|e| e.to_string())?;
                return crate::tv::wake_tv(&settings, &path);
            }
            if !tv.contains_key(connection) {
                let settings = couch_webos::Settings::load(&connections::file(connection, "webos"))
                    .map_err(|e| e.to_string())?;
                tv.insert(
                    connection.into(),
                    couch_webos::Client::connect(&settings).map_err(|e| e.to_string())?,
                );
            }
            let result = crate::tv::mapped_command(tv.get_mut(connection).unwrap(), command);
            if result.is_err() {
                tv.remove(connection);
            }
            result.map_err(|e| e.to_string())
        }
        Integration::Hue { light_id } => {
            let (id, raw) = connections::split(&light_id);
            let c = couch_hue::settings::Settings::load(&connections::file(id, "hue"))
                .and_then(|s| s.client())
                .map_err(|e| e.to_string())?;
            let on = match command {
                "on" => true,
                "off" => false,
                _ => !c
                    .control_state(raw)
                    .map_err(|e| e.to_string())?
                    .on
                    .ok_or("Hue light is unavailable")?,
            };
            c.set_power(raw, on).map_err(|e| e.to_string())
        }
        Integration::HomeAssistant { entity_id } => {
            let (c, raw) = connections::ha(&entity_id)?;
            let on = match command {
                "on" => true,
                "off" => false,
                _ => !c
                    .light(&raw)
                    .map_err(|e| e.to_string())?
                    .on
                    .ok_or("Home Assistant light is unavailable")?,
            };
            c.command(
                &raw,
                if on {
                    couch_ha::Command::On
                } else {
                    couch_ha::Command::Off
                },
            )
            .map_err(|e| e.to_string())
        }
        _ => Err("This integration cannot send button commands yet".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Controller, mpsc::Receiver<Request>) {
        let (tx, rx) = mpsc::sync_channel(8);
        let (_, out) = mpsc::channel();
        (
            Controller {
                context: "watch".into(),
                config: Arc::new(Config::default()),
                bindings: vec![],
                pending: HashMap::new(),
                replay: VecDeque::new(),
                generation: Arc::new(AtomicU64::new(1)),
                tx,
                rx: out,
            },
            rx,
        )
    }
    fn press(code: u16, released: bool) -> Press {
        Press {
            code,
            released,
            key: None,
            mic: None,
            menu: None,
            latency_us: 0,
            repeat: false,
        }
    }
    fn binding(gesture: Gesture, command: &str) -> Binding {
        Binding {
            button: Button::Ok,
            gesture,
            action: Some(Action::new("player", command)),
        }
    }
    #[test]
    fn short_and_long_are_mutually_exclusive_even_across_a_slow_frame() {
        let (mut c, rx) = fixture();
        c.bindings = vec![
            binding(Gesture::Short, "ok"),
            binding(Gesture::Long, "home"),
        ];
        assert!(c.handle_press(&press(353, false)));
        assert!(rx.try_recv().is_err());
        c.handle_press(&press(353, true));
        assert_eq!(rx.try_recv().unwrap().action.command, "ok");
        assert!(rx.try_recv().is_err());
        c.handle_press(&press(353, false));
        c.pending.get_mut(&353).unwrap().at = Instant::now() - HOLD;
        c.handle_press(&press(353, true));
        assert_eq!(rx.try_recv().unwrap().action.command, "home");
        assert!(rx.try_recv().is_err());
        c.handle_press(&press(353, false));
        c.pending.get_mut(&353).unwrap().fired = true;
        c.fire(Button::Ok, Gesture::Long, false);
        c.handle_press(&press(353, true));
        assert_eq!(rx.try_recv().unwrap().action.command, "home");
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn only_a_short_tap_replays_default_when_long_is_overridden() {
        let (mut c, rx) = fixture();
        c.bindings = vec![binding(Gesture::Long, "home")];
        c.handle_press(&press(353, false));
        c.handle_press(&press(353, true));
        assert_eq!(c.replay.len(), 2);
        assert!(rx.try_recv().is_err());
        c.replay.clear();
        c.handle_press(&press(353, false));
        c.pending.get_mut(&353).unwrap().at = Instant::now() - HOLD;
        c.handle_press(&press(353, true));
        assert!(c.replay.is_empty());
        assert_eq!(rx.try_recv().unwrap().action.command, "home");
    }
    #[test]
    fn disabled_buttons_and_nonrepeatable_actions_do_not_send_commands() {
        let (mut c, rx) = fixture();
        c.bindings = vec![Binding {
            button: Button::VolumeUp,
            gesture: Gesture::Short,
            action: None,
        }];
        assert!(c.handle_press(&press(115, false)));
        assert!(rx.try_recv().is_err());
        c.bindings[0].action = Some(Action::new("tv", "mute"));
        let mut p = press(115, false);
        p.repeat = true;
        c.handle_press(&p);
        assert!(rx.try_recv().is_err());
        p.repeat = false;
        c.handle_press(&p);
        assert_eq!(rx.try_recv().unwrap().action.command, "mute");
    }
}
