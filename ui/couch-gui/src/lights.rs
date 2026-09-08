//! Room light controls. Network requests run on one worker, never on Slint's thread.
use crate::{home, App, ChoiceItem};
use couch_ha::{settings::Settings, Command, Light};
use couch_model::{Config, DeviceKind, Id, Integration};
use slint::{Model, ModelRc, VecModel};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
/// Short-lived observations speed up navigation, never authorize commands.
#[derive(Default)]
struct StateCache(HashMap<String, (Instant, Light)>);
impl StateCache {
    fn get(&self, id: &str) -> Option<Light> {
        self.0
            .get(id)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(5))
            .map(|(_, s)| s.clone())
    }
    fn put(&mut self, state: Light) {
        self.0
            .retain(|_, (at, _)| at.elapsed() < Duration::from_secs(5));
        self.0
            .insert(state.entity_id.clone(), (Instant::now(), state));
    }
}
#[derive(Clone)]
struct Entry {
    icon: couch_model::Icon,
    name: String,
    id: String,
    state: Option<Light>,
    hue: bool,
}
enum Operation {
    List,
    Toggle(String),
    Brightness(String, u8),
}
enum Answer {
    List(Vec<Entry>),
    State(Light),
}
enum Input {
    Open(Id),
    Pick(usize),
    Brightness(usize, i32),
    Back,
}
pub struct Controller {
    input: Rc<RefCell<VecDeque<Input>>>,
    tx: mpsc::SyncSender<(u64, Id, Operation)>,
    rx: mpsc::Receiver<(u64, Result<Answer, String>)>,
    generation: u64,
    room: Option<Id>,
    entries: Vec<Entry>,
    cache: StateCache,
    hue: Arc<couch_hue::live::Live>,
    busy: Option<String>,
    refreshing: bool,
    last_refresh: Instant,
    brightness_pending: VecDeque<(String, u8)>,
    brightness_flight: Option<(String, u8)>,
    brightness_until: Option<Instant>,
    last_brightness_send: Instant,
}
fn configured(room: &Id) -> Result<Vec<Entry>, String> {
    let config: Config = serde_json::from_slice(
        &std::fs::read(home::path("config.json")).map_err(|_| "Cannot read your rooms")?,
    )
    .map_err(|_| "Cannot read your rooms")?;
    let room = config
        .room(room)
        .ok_or("This room was removed; return home to reload")?;
    Ok(room
        .devices
        .iter()
        .filter_map(
            |d| match config.resolve_integration(&d.integration).as_ref() {
                Some(Integration::HomeAssistant { entity_id }) if d.kind == DeviceKind::Light => {
                    Some(Entry {
                        name: d.name.clone(),
                        icon: d.effective_icon(),
                        id: entity_id.clone(),
                        state: None,
                        hue: false,
                    })
                }
                Some(Integration::Hue { light_id }) if !light_id.starts_with("scene:") => {
                    Some(Entry {
                        name: d.name.clone(),
                        icon: d.effective_icon(),
                        id: format!("hue:{light_id}"),
                        state: None,
                        hue: true,
                    })
                }
                _ => Some(Entry {
                    name: d.name.clone(),
                    icon: d.effective_icon(),
                    id: format!("device:{}", d.id),
                    state: None,
                    hue: false,
                }),
            },
        )
        .collect())
}
fn toggle_command(state: &Light) -> Result<Command, String> {
    match state.on {
        Some(true) => Ok(Command::Off),
        Some(false) => Ok(Command::On),
        None => Err("This light is unavailable".into()),
    }
}
fn perform(room: &Id, operation: Operation, hue: &couch_hue::live::Live) -> Result<Answer, String> {
    let mut entries = configured(room)?;
    let ha = || {
        Settings::load(&home::path("ha-connection.json"))
            .and_then(|s| s.client())
            .map_err(|e| e.to_string())
    };
    match operation {
        Operation::List => {
            let ha_states = if entries
                .iter()
                .any(|e| !e.hue && !e.id.starts_with("device:"))
            {
                ha().and_then(|c| c.lights().map_err(|e| e.to_string()))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let hue_states = if entries.iter().any(|e| e.hue) {
                hue.lights().unwrap_or_default()
            } else {
                Vec::new()
            };
            for e in &mut entries {
                let states = if e.hue { &hue_states } else { &ha_states };
                let id = e.id.strip_prefix("hue:").unwrap_or(&e.id);
                e.state = states
                    .iter()
                    .find(|s| s.entity_id == id)
                    .cloned()
                    .map(|mut s| {
                        s.entity_id = e.id.clone();
                        s
                    });
            }
            Ok(Answer::List(entries))
        }
        Operation::Brightness(id, percent) => {
            if !entries.iter().any(|e| e.id == id) {
                return Err("This device was removed from the room".into());
            }
            let mut state = if let Some(raw) = id.strip_prefix("hue:") {
                hue.brightness(raw, percent).map_err(|e| e.to_string())?
            } else if id.starts_with("device:") {
                return Err("This device does not support brightness".into());
            } else {
                let c = ha()?;
                c.command(&id, Command::Brightness(percent))
                    .map_err(|e| e.to_string())?;
                c.light(&id).map_err(|e| e.to_string())?
            };
            state.entity_id = id;
            Ok(Answer::State(state))
        }
        Operation::Toggle(id) => {
            if !entries.iter().any(|e| e.id == id) {
                return Err("This device was removed from the room".into());
            }
            // Hue uses its push-maintained cache; HA still reads before toggling.
            let mut state = if let Some(raw) = id.strip_prefix("hue:") {
                let started = Instant::now();
                let result = hue.toggle(raw).map_err(|e| e.to_string());
                println!(
                    "couch-gui: Hue toggle acknowledged in {} ms (success={})",
                    started.elapsed().as_millis(),
                    result.is_ok()
                );
                result?
            } else if id.starts_with("device:") {
                return Err("Controls for this device are not available yet".into());
            } else {
                let c = ha()?;
                let state = c.light(&id).map_err(|e| e.to_string())?;
                c.command(&id, toggle_command(&state)?)
                    .map_err(|e| e.to_string())?;
                c.light(&id).map_err(|e| e.to_string())?
            };
            state.entity_id = id;
            Ok(Answer::State(state))
        }
    }
}

impl Controller {
    pub fn install(app: &App) -> Self {
        let input = Rc::new(RefCell::new(VecDeque::new()));
        let q = input.clone();
        app.on_light_activate(move |i| {
            if i >= 0 {
                q.borrow_mut().push_back(Input::Pick(i as usize));
            }
        });
        let q = input.clone();
        app.on_light_brightness(move |i, delta| {
            if i >= 0 {
                q.borrow_mut()
                    .push_back(Input::Brightness(i as usize, delta));
            }
        });
        let q = input.clone();
        app.on_light_back(move || q.borrow_mut().push_back(Input::Back));
        let (tx, requests) = mpsc::sync_channel::<(u64, Id, Operation)>(1);
        let (events, rx) = mpsc::channel();
        let hue = Arc::new(couch_hue::live::Live::new(home::path(
            "hue-connection.json",
        )));
        let worker_hue = hue.clone();
        std::thread::spawn(move || {
            let _ = worker_hue.lights(); // Warm the cache without delaying GUI startup.
            while let Ok((generation, room, op)) = requests.recv() {
                let _ = events.send((generation, perform(&room, op, &worker_hue)));
            }
        });
        Self {
            input,
            tx,
            rx,
            generation: 0,
            room: None,
            entries: Vec::new(),
            cache: StateCache::default(),
            hue,
            busy: None,
            refreshing: false,
            last_refresh: Instant::now(),
            brightness_pending: VecDeque::new(),
            brightness_flight: None,
            brightness_until: None,
            last_brightness_send: Instant::now() - Duration::from_secs(1),
        }
    }
    pub fn hue_live(&self) -> Arc<couch_hue::live::Live> {
        self.hue.clone()
    }
    pub fn wake(&mut self) {
        self.hue.reset();
        self.cache.0.retain(|id, _| !id.starts_with("hue:"));
        self.last_refresh = Instant::now() - Duration::from_secs(5);
    }
    pub fn opener(&self) -> impl Fn(Id) + 'static {
        let input = self.input.clone();
        move |room| input.borrow_mut().push_back(Input::Open(room))
    }
    fn row(&self, e: &Entry) -> ChoiceItem {
        let detail = if self.busy.as_deref() == Some(&e.id) {
            "Updating…".into()
        } else if e.id.starts_with("device:") {
            "Controls not available yet".into()
        } else {
            e.state.as_ref().map(description).unwrap_or_else(|| {
                if self.refreshing {
                    "Checking status…".into()
                } else {
                    "Unavailable".into()
                }
            })
        };
        ChoiceItem {
            icon: crate::icons::image(e.icon),
            title: e.name.clone().into(),
            detail: detail.into(),
            light: !e.id.starts_with("device:"),
            active: e.state.as_ref().is_some_and(|s| s.on == Some(true)),
            power_known: e.state.as_ref().is_some_and(|s| s.on.is_some()),
        }
    }
    fn update_rows(&self, app: &App, reset: bool) {
        if !reset {
            let model = app.get_light_items();
            if let Some(rows) = model.as_any().downcast_ref::<VecModel<ChoiceItem>>() {
                if rows.row_count() == self.entries.len() {
                    for (i, e) in self.entries.iter().enumerate() {
                        rows.set_row_data(i, self.row(e));
                    }
                    return;
                }
            }
        }
        app.set_light_items(ModelRc::new(VecModel::from(
            self.entries.iter().map(|e| self.row(e)).collect::<Vec<_>>(),
        )));
    }
    fn refresh(&mut self) {
        let Some(room) = self.room.clone() else {
            return;
        };
        self.last_refresh = Instant::now();
        self.refreshing = self
            .tx
            .try_send((self.generation, room, Operation::List))
            .is_ok();
    }
    fn clear_brightness(&mut self, app: &App) {
        self.brightness_pending.clear();
        self.brightness_flight = None;
        self.brightness_until = None;
        app.set_brightness_shown(false);
    }
    fn adjust_brightness(&mut self, app: &App, i: usize, delta: i32) {
        let Some(entry) = self.entries.get(i) else {
            return;
        };
        let Some(state) = &entry.state else {
            app.set_light_detail("Checking this light’s status. Try again in a moment.".into());
            return;
        };
        let target = self
            .brightness_pending
            .iter()
            .find(|(id, _)| id == &entry.id)
            .or_else(|| {
                self.brightness_flight
                    .as_ref()
                    .filter(|(id, _)| id == &entry.id)
            })
            .map(|(_, p)| *p);
        match brightness_step(state, target, delta) {
            Ok(percent) => {
                queue_brightness(&mut self.brightness_pending, entry.id.clone(), percent);
                app.set_brightness_target(entry.name.clone().into());
                app.set_light_brightness_percent(percent as i32);
                app.set_brightness_shown(true);
                app.set_light_detail("".into());
                self.brightness_until = Some(Instant::now() + Duration::from_secs(2));
            }
            Err(error) => {
                app.set_light_detail(error.into());
            }
        }
    }
    fn send_brightness(&mut self) {
        if self.busy.is_some()
            || self.refreshing
            || self.last_brightness_send.elapsed() < Duration::from_millis(100)
        {
            return;
        }
        let Some(room) = self.room.clone() else {
            return;
        };
        let Some((id, percent)) = self.brightness_pending.front().cloned() else {
            return;
        };
        let generation = self.generation + 1;
        if self
            .tx
            .try_send((generation, room, Operation::Brightness(id.clone(), percent)))
            .is_ok()
        {
            self.generation = generation;
            self.busy = Some(id.clone());
            self.brightness_flight = Some((id, percent));
            self.brightness_pending.pop_front();
            self.last_brightness_send = Instant::now();
        }
    }
    fn open_room(&mut self, app: &App, room: Id) {
        self.clear_brightness(app);
        let started = Instant::now();
        self.generation += 1;
        self.room = Some(room.clone());
        self.busy = None;
        let title = std::fs::read(home::path("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
            .and_then(|c| c.room(&room).map(|r| r.name.clone()))
            .unwrap_or_else(|| "Room".into());
        app.set_light_title(title.into());
        app.set_light_room_id(room.as_str().into());
        let scene_names = std::fs::read(home::path("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
            .map(|c| {
                c.scenes
                    .iter()
                    .filter(|s| s.rooms.contains(&room))
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let scenes = scene_names.len();
        app.set_light_scene_label(if scenes == 1 {
            scene_names[0].clone().into()
        } else {
            format!("{scenes} scenes").into()
        });
        app.set_light_scene_count(scenes as i32);
        app.set_light_detail("".into());
        app.set_light_shown(true);
        match configured(&room) {
            Ok(mut entries) => {
                for e in &mut entries {
                    e.state = self.cache.get(&e.id);
                }
                self.entries = entries;
                self.refresh();
                if self.entries.is_empty() {
                    app.set_light_detail("Add devices to this room in the web editor.".into());
                }
            }
            Err(error) => {
                self.entries.clear();
                self.refreshing = false;
                app.set_light_detail(error.into());
            }
        }
        self.update_rows(app, true);
        app.invoke_focus_light();
        println!(
            "couch-gui: room list ready in {} us ({} devices)",
            started.elapsed().as_micros(),
            self.entries.len()
        );
    }
    pub fn navigation_pending(&self) -> bool {
        self.input
            .borrow()
            .iter()
            .any(|input| matches!(input, Input::Open(_) | Input::Back))
    }
    pub fn poll(&mut self, app: &App) {
        if self
            .brightness_until
            .is_some_and(|until| Instant::now() >= until)
        {
            self.brightness_until = None;
            app.set_brightness_shown(false);
        }
        if app.get_pair_shown() {
            self.input.borrow_mut().clear();
            self.clear_brightness(app);
            return;
        }
        loop {
            let Some(input) = self.input.borrow_mut().pop_front() else {
                break;
            };
            match input {
                Input::Open(room) => self.open_room(app, room),
                Input::Back => {
                    self.clear_brightness(app);
                    self.generation += 1;
                    self.room = None;
                    self.busy = None;
                    self.refreshing = false;
                    app.set_light_shown(false);
                    app.invoke_focus_home();
                }
                Input::Brightness(i, delta) => {
                    if self.room.is_some() {
                        self.adjust_brightness(app, i, delta);
                    }
                }
                Input::Pick(i) => {
                    if self.room.is_none()
                        || self.busy.is_some()
                        || !self.brightness_pending.is_empty()
                    {
                        continue;
                    }
                    let Some(e) = self.entries.get(i) else {
                        continue;
                    };
                    if e.id.starts_with("device:") {
                        app.set_light_detail(
                            "Controls for this device are not available yet.".into(),
                        );
                        continue;
                    }
                    let id = e.id.clone();
                    let generation = self.generation + 1;
                    if self
                        .tx
                        .try_send((
                            generation,
                            self.room.clone().unwrap(),
                            Operation::Toggle(id.clone()),
                        ))
                        .is_ok()
                    {
                        self.generation = generation;
                        self.busy = Some(id);
                        self.refreshing = false;
                        app.set_light_detail("".into());
                        self.update_rows(app, false);
                    } else {
                        app.set_light_detail("Connection busy. Press OK again in a moment.".into());
                    }
                }
            }
        }
        while let Ok((generation, result)) = self.rx.try_recv() {
            if generation != self.generation || self.room.is_none() {
                continue;
            }
            self.last_refresh = Instant::now();
            self.refreshing = false;
            self.busy = None;
            let brightness = self.brightness_flight.take();
            match result {
                Ok(Answer::List(entries)) => {
                    let reset = self.entries.len() != entries.len()
                        || self.entries.iter().zip(&entries).any(|(a, b)| a.id != b.id);
                    for e in &entries {
                        if let Some(s) = &e.state {
                            self.cache.put(s.clone());
                        } else {
                            self.cache.0.remove(&e.id);
                        }
                    }
                    self.entries = entries;
                    self.update_rows(app, reset);
                }
                Ok(Answer::State(s)) => {
                    self.cache.put(s.clone());
                    for e in &mut self.entries {
                        if e.id == s.entity_id {
                            e.state = Some(s.clone());
                        }
                    }
                    self.update_rows(app, false);
                    app.set_light_detail("".into());
                }
                Err(error) => {
                    if let Some((id, _)) = brightness {
                        self.brightness_pending
                            .retain(|(pending, _)| pending != &id);
                        app.set_brightness_shown(false);
                    }
                    app.set_light_detail(error.into());
                    self.update_rows(app, false);
                }
            }
        }
        self.send_brightness();
        if self.room.is_some()
            && self.brightness_pending.is_empty()
            && self.busy.is_none()
            && !self.refreshing
            && self.last_refresh.elapsed()
                > Duration::from_millis(if self.entries.iter().all(|e| e.hue) {
                    500
                } else {
                    5000
                })
        {
            self.refresh();
        }
    }
}
// Retain only the latest unsent level per device, preserving device order.
fn queue_brightness(queue: &mut VecDeque<(String, u8)>, id: String, percent: u8) {
    if let Some((_, target)) = queue.iter_mut().find(|(pending, _)| pending == &id) {
        *target = percent;
    } else {
        queue.push_back((id, percent));
    }
}
fn brightness_step(light: &Light, target: Option<u8>, delta: i32) -> Result<u8, &'static str> {
    if light.on.is_none() {
        return Err("This light is unavailable.");
    }
    if !light.dimmable {
        return Err("This light does not support brightness.");
    }
    let current = target
        .or_else(|| {
            if light.on == Some(false) {
                Some(0)
            } else {
                light.brightness_percent
            }
        })
        .ok_or("Checking brightness. Try again in a moment.")?;
    Ok((current as i32 + delta.clamp(-100, 100)).clamp(0, 100) as u8)
}
fn description(light: &Light) -> String {
    match light.on {
        None => "Unavailable".into(),
        Some(false) => "Off".into(),
        Some(true) => light
            .brightness_percent
            .map(|p| format!("On · {p}%"))
            .unwrap_or_else(|| "On".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brightness_steps_use_pending_targets_clamp_and_reject_unsupported_lights() {
        let mut light = Light {
            entity_id: "light.test".into(),
            name: "Test".into(),
            on: Some(true),
            brightness_percent: Some(50),
            dimmable: true,
        };
        assert_eq!(brightness_step(&light, None, 5), Ok(55));
        assert_eq!(brightness_step(&light, Some(55), 5), Ok(60));
        assert_eq!(brightness_step(&light, Some(98), 5), Ok(100));
        assert_eq!(brightness_step(&light, Some(2), -5), Ok(0));
        light.on = Some(false);
        assert_eq!(brightness_step(&light, None, 5), Ok(5));
        light.on = None;
        assert!(brightness_step(&light, Some(50), 5).is_err());
        light.on = Some(true);
        light.dimmable = false;
        assert!(brightness_step(&light, None, 5).is_err());
        light.dimmable = true;
        light.brightness_percent = None;
        assert!(brightness_step(&light, None, 5).is_err());
    }
    #[test]
    fn rapid_dimming_keeps_latest_target_without_dropping_other_lights() {
        let mut queue = VecDeque::new();
        queue_brightness(&mut queue, "one".into(), 55);
        queue_brightness(&mut queue, "two".into(), 25);
        queue_brightness(&mut queue, "one".into(), 60);
        queue_brightness(&mut queue, "one".into(), 65);
        assert_eq!(
            queue.into_iter().collect::<Vec<_>>(),
            vec![("one".into(), 65), ("two".into(), 25)]
        );
    }
    #[test]
    fn toggle_uses_live_state_and_rejects_unavailable() {
        let mut state = Light {
            entity_id: "light.test".into(),
            name: "Test".into(),
            on: Some(false),
            brightness_percent: None,
            dimmable: true,
        };
        assert!(matches!(toggle_command(&state), Ok(Command::On)));
        state.on = Some(true);
        assert!(matches!(toggle_command(&state), Ok(Command::Off)));
        state.on = None;
        assert!(toggle_command(&state).is_err());
    }
    #[test]
    fn navigation_cache_expires() {
        let mut c = StateCache::default();
        c.put(Light {
            entity_id: "light.test".into(),
            name: "Test".into(),
            on: Some(true),
            brightness_percent: None,
            dimmable: true,
        });
        assert!(c.get("light.test").is_some());
        c.0.get_mut("light.test").unwrap().0 = Instant::now() - Duration::from_secs(6);
        assert!(c.get("light.test").is_none());
    }
}
