//! Room light controls. Network requests run on one worker, never on Slint's thread.
use crate::{home, App, ChoiceItem};
use couch_ha::{settings::Settings, Command, Light};
use couch_model::{Config, DeviceKind, Id, Integration};
use slint::{Model, ModelRc, VecModel};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::mpsc,
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
    name: String,
    id: String,
    state: Option<Light>,
    hue: bool,
}
enum Operation {
    List,
    State(String),
    RefreshState(String),
    Send(String, Command),
}
enum Answer {
    List(Vec<Entry>),
    State(Light, bool),
}
enum Input {
    Open(Id),
    Pick(usize),
    Back,
}
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Closed,
    Loading,
    List,
    Light,
    Brightness,
    Error,
}
pub struct Controller {
    input: Rc<RefCell<VecDeque<Input>>>,
    tx: mpsc::SyncSender<(u64, Id, Operation)>,
    rx: mpsc::Receiver<(u64, Result<Answer, String>)>,
    generation: u64,
    room: Option<Id>,
    page: Page,
    entries: Vec<Entry>,
    selected: Option<Light>,
    cache: StateCache,
    refreshing: bool,
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
                        id: entity_id.clone(),
                        state: None,
                        hue: false,
                    })
                }
                Some(Integration::Hue { light_id }) if d.kind == DeviceKind::Light => Some(Entry {
                    name: d.name.clone(),
                    id: format!("hue:{light_id}"),
                    state: None,
                    hue: true,
                }),
                _ => None,
            },
        )
        .collect())
}
fn perform(room: &Id, operation: Operation, cache: &StateCache) -> Result<Answer, String> {
    let mut entries = configured(room)?;
    if matches!(operation, Operation::List) && entries.is_empty() {
        return Ok(Answer::List(entries));
    }
    if let Operation::State(id) = &operation {
        if entries.iter().any(|e| e.id == *id) {
            if let Some(state) = cache.get(id) {
                return Ok(Answer::State(state, false));
            }
        }
    }
    let ha = || {
        Settings::load(&home::path("ha-connection.json"))
            .and_then(|s| s.client())
            .map_err(|_| "Set up Home Assistant under Connections in the web editor".to_string())
    };
    let hue = || {
        couch_hue::settings::Settings::load(&home::path("hue-connection.json"))
            .and_then(|s| s.client())
            .map_err(|_| "Pair Philips Hue under Connections in the web editor".to_string())
    };
    match operation {
        Operation::List => {
            // One unavailable integration must not hide the other bridge's lights.
            let ha_states = if entries.iter().any(|e| !e.hue) {
                ha().and_then(|c| c.lights().map_err(|e| e.to_string()))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let hue_states = if entries.iter().any(|e| e.hue) {
                hue()
                    .and_then(|c| c.lights().map_err(|e| e.to_string()))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            for entry in &mut entries {
                let states = if entry.hue { &hue_states } else { &ha_states };
                let id = entry.id.strip_prefix("hue:").unwrap_or(&entry.id);
                entry.state = states
                    .iter()
                    .find(|s| s.entity_id == id)
                    .cloned()
                    .map(|mut s| {
                        s.entity_id = entry.id.clone();
                        s
                    });
            }
            Ok(Answer::List(entries))
        }
        Operation::State(id) | Operation::RefreshState(id) | Operation::Send(id, _)
            if !entries.iter().any(|e| e.id == id) =>
        {
            Err("This light was removed from the room".into())
        }
        op => {
            let (id, command) = match op {
                Operation::State(id) | Operation::RefreshState(id) => (id, None),
                Operation::Send(id, c) => (id, Some(c)),
                _ => unreachable!(),
            };
            let mut state = if let Some(raw) = id.strip_prefix("hue:") {
                let c = hue()?;
                if let Some(cmd) = command {
                    c.command(raw, cmd).map_err(|e| e.to_string())?;
                }
                c.light(raw).map_err(|e| e.to_string())?
            } else {
                let c = ha()?;
                if let Some(cmd) = command {
                    c.command(&id, cmd).map_err(|e| e.to_string())?;
                }
                c.light(&id).map_err(|e| e.to_string())?
            };
            state.entity_id = id;
            Ok(Answer::State(state, command.is_some()))
        }
    }
}

impl Controller {
    pub fn install(app: &App) -> Self {
        let input = Rc::new(RefCell::new(VecDeque::new()));
        let queue = input.clone();
        app.on_light_activate(move |i| {
            if i >= 0 {
                queue.borrow_mut().push_back(Input::Pick(i as usize));
            }
        });
        let queue = input.clone();
        app.on_light_back(move || queue.borrow_mut().push_back(Input::Back));
        let (tx, requests) = mpsc::sync_channel::<(u64, Id, Operation)>(1);
        let (events, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut cache = StateCache::default();
            while let Ok((generation, room, operation)) = requests.recv() {
                let answer = perform(&room, operation, &cache);
                match &answer {
                    Ok(Answer::List(entries)) => {
                        for e in entries {
                            if let Some(s) = &e.state {
                                cache.put(s.clone())
                            }
                        }
                    }
                    Ok(Answer::State(s, _)) => cache.put(s.clone()),
                    _ => {}
                }
                let _ = events.send((generation, answer));
            }
        });
        Self {
            input,
            tx,
            rx,
            generation: 0,
            room: None,
            page: Page::Closed,
            entries: Vec::new(),
            selected: None,
            cache: StateCache::default(),
            refreshing: false,
        }
    }
    pub fn opener(&self) -> impl Fn(Id) + 'static {
        let input = self.input.clone();
        move |room| input.borrow_mut().push_back(Input::Open(room))
    }
    fn page(&self, app: &App, title: &str, detail: &str, items: Vec<(String, String)>) {
        app.set_light_shown(true);
        app.set_light_title(title.into());
        app.set_light_detail(detail.into());
        app.set_light_items(ModelRc::new(VecModel::from(
            items
                .into_iter()
                .map(|(title, detail)| ChoiceItem {
                    title: title.into(),
                    detail: detail.into(),
                    active: false,
                })
                .collect::<Vec<_>>(),
        )));
        app.invoke_focus_light();
    }
    fn buttons(&self, app: &App, title: &str, detail: &str, labels: &[&str]) {
        self.page(
            app,
            title,
            detail,
            labels
                .iter()
                .map(|s| (s.to_string(), String::new()))
                .collect(),
        );
    }
    fn request(&mut self, app: &App, operation: Operation) {
        let Some(room) = self.room.clone() else {
            return;
        };
        self.generation += 1;
        self.refreshing = false;
        self.page = Page::Loading;
        if self
            .tx
            .try_send((self.generation, room, operation))
            .is_err()
        {
            self.page = Page::Error;
            self.buttons(
                app,
                "Connection busy",
                "A previous request is still finishing.",
                &["Retry", "Back"],
            );
            return;
        }
        self.buttons(
            app,
            "Connecting",
            "Waiting for the light connection…",
            &["Back"],
        );
    }
    fn list(&mut self, app: &App) {
        self.page = Page::List;
        let mut items: Vec<_> = self
            .entries
            .iter()
            .map(|e| {
                (
                    e.name.clone(),
                    e.state.as_ref().map(description).unwrap_or_else(|| {
                        if self.refreshing {
                            "Checking status…".into()
                        } else {
                            "Unavailable — select for details".into()
                        }
                    }),
                )
            })
            .collect();
        items.extend([
            ("Refresh".into(), String::new()),
            ("Back".into(), String::new()),
        ]);
        self.page(
            app,
            "Room lights",
            if self.entries.is_empty() {
                "Add Hue or Home Assistant lights to this room in the web editor."
            } else if self.refreshing {
                "Updating status in the background…"
            } else {
                "Choose a light to control it."
            },
            items,
        );
    }
    fn open_room(&mut self, app: &App, room: Id) {
        let started = Instant::now();
        self.room = Some(room.clone());
        self.generation += 1;
        self.selected = None;
        match configured(&room) {
            Ok(mut entries) => {
                for e in &mut entries {
                    e.state = self.cache.get(&e.id);
                }
                self.entries = entries;
                self.refreshing = !self.entries.is_empty()
                    && self
                        .tx
                        .try_send((self.generation, room, Operation::List))
                        .is_ok();
                self.list(app);
                println!(
                    "couch-gui: room list ready in {} us ({} lights), status refresh {}",
                    started.elapsed().as_micros(),
                    self.entries.len(),
                    self.refreshing
                );
            }
            Err(error) => {
                self.page = Page::Error;
                self.buttons(app, "Cannot open room", &error, &["Retry", "Back"]);
            }
        }
    }
    fn accept_list(&mut self, app: &App, entries: Vec<Entry>) {
        self.refreshing = false;
        for e in &entries {
            if let Some(s) = &e.state {
                self.cache.put(s.clone());
            }
        }
        let same = self.page == Page::List
            && self.entries.len() == entries.len()
            && self.entries.iter().zip(&entries).all(|(a, b)| a.id == b.id);
        self.entries = entries;
        if same {
            // Update rows in place: replacing the model would reset D-pad focus
            // while the user is already navigating the instantly visible list.
            let model = app.get_light_items();
            if let Some(rows) = model.as_any().downcast_ref::<VecModel<ChoiceItem>>() {
                for (i, e) in self.entries.iter().enumerate() {
                    rows.set_row_data(
                        i,
                        ChoiceItem {
                            title: e.name.clone().into(),
                            detail: e
                                .state
                                .as_ref()
                                .map(description)
                                .unwrap_or_else(|| "Unavailable — select for details".into())
                                .into(),
                            active: false,
                        },
                    );
                }
                app.set_light_detail("Choose a light to control it.".into());
                return;
            }
        }
        self.list(app);
    }
    fn light(&mut self, app: &App, notice: &str) {
        let Some(light) = self.selected.as_ref() else {
            return;
        };
        self.page = Page::Light;
        let labels = if light.on.is_none() {
            vec!["Refresh", "Back"]
        } else if light.dimmable {
            vec!["Turn on", "Turn off", "Set brightness", "Refresh", "Back"]
        } else {
            vec!["Turn on", "Turn off", "Refresh", "Back"]
        };
        self.buttons(
            app,
            &light.name,
            &format!("{}\n{notice}", description(light)),
            &labels,
        );
    }
    fn close(&mut self, app: &App) {
        self.generation += 1;
        self.page = Page::Closed;
        self.room = None;
        self.selected = None;
        app.set_light_shown(false);
        app.invoke_focus_home();
    }
    pub fn poll(&mut self, app: &App) {
        if app.get_pair_shown() {
            self.input.borrow_mut().clear();
            return;
        }
        loop {
            let action = self.input.borrow_mut().pop_front();
            let Some(action) = action else { break };
            match action {
                Input::Open(room) => {
                    self.open_room(app, room);
                }
                Input::Back => match self.page {
                    Page::Light | Page::Brightness => self.list(app),
                    _ => self.close(app),
                },
                Input::Pick(i) => match self.page {
                    Page::List => {
                        if let Some(entry) = self.entries.get(i) {
                            if let Some(state) = self.cache.get(&entry.id) {
                                self.generation += 1;
                                self.refreshing = false;
                                self.selected = Some(state);
                                self.light(app, "");
                            } else {
                                self.request(app, Operation::State(entry.id.clone()));
                            }
                        } else if i == self.entries.len() {
                            if let Some(room) = self.room.clone() {
                                self.open_room(app, room);
                            }
                        } else {
                            self.close(app)
                        }
                    }
                    Page::Loading => self.close(app),
                    Page::Error => {
                        if i == 0 {
                            self.request(app, Operation::List)
                        } else {
                            self.close(app)
                        }
                    }
                    Page::Brightness => {
                        if i < 10 {
                            if let Some(light) = &self.selected {
                                self.request(
                                    app,
                                    Operation::Send(
                                        light.entity_id.clone(),
                                        Command::Brightness(((i + 1) * 10) as u8),
                                    ),
                                );
                            }
                        } else {
                            self.light(app, "");
                        }
                    }
                    Page::Light => {
                        let Some(light) = self.selected.clone() else {
                            continue;
                        };
                        if light.on.is_none() {
                            if i == 0 {
                                self.request(app, Operation::RefreshState(light.entity_id));
                            } else {
                                self.list(app);
                            }
                            continue;
                        }
                        match i {
                            0 => self.request(app, Operation::Send(light.entity_id, Command::On)),
                            1 => self.request(app, Operation::Send(light.entity_id, Command::Off)),
                            2 if light.dimmable => {
                                self.page = Page::Brightness;
                                let mut rows: Vec<_> = (1..=10)
                                    .map(|n| (format!("{}%", n * 10), String::new()))
                                    .collect();
                                rows.push(("Back".into(), String::new()));
                                self.page(app, "Set brightness", &light.name, rows);
                            }
                            n if n == if light.dimmable { 3 } else { 2 } => {
                                self.request(app, Operation::RefreshState(light.entity_id))
                            }
                            _ => self.list(app),
                        }
                    }
                    _ => {}
                },
            }
        }
        while let Ok((generation, result)) = self.rx.try_recv() {
            if generation != self.generation || self.page == Page::Closed {
                continue;
            }
            match result {
                Ok(Answer::List(entries)) => {
                    self.accept_list(app, entries);
                }
                Ok(Answer::State(state, sent)) => {
                    self.cache.put(state.clone());
                    for e in &mut self.entries {
                        if e.id == state.entity_id {
                            e.state = Some(state.clone());
                        }
                    }
                    self.selected = Some(state);
                    self.light(
                        app,
                        if sent {
                            "Request sent. Refresh for the latest state."
                        } else {
                            ""
                        },
                    );
                }
                Err(error) => {
                    self.page = Page::Error;
                    self.buttons(app, "Light control unavailable", &error, &["Retry", "Back"]);
                }
            }
        }
    }
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
    fn navigation_cache_expires_and_does_not_mix_integrations() {
        let mut cache = StateCache::default();
        let state = Light {
            entity_id: "hue:one".into(),
            name: "Light".into(),
            on: Some(true),
            brightness_percent: Some(50),
            dimmable: true,
        };
        cache.put(state.clone());
        assert_eq!(cache.get("hue:one"), Some(state));
        assert!(cache.get("light.one").is_none());
        cache.0.get_mut("hue:one").unwrap().0 = Instant::now() - Duration::from_secs(6);
        assert!(cache.get("hue:one").is_none());
    }
}
