//! Room light controls. Network requests run on one worker, never on Slint's thread.
use crate::{home, App, ChoiceItem};
use couch_ha::{settings::Settings, Command, Light};
use couch_model::{Config, DeviceKind, Id, Integration};
use slint::{ModelRc, VecModel};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::mpsc};
#[derive(Clone)]
struct Entry {
    name: String,
    id: String,
    state: Option<Light>,
}
enum Operation {
    List,
    State(String),
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
        .filter_map(|d| match &d.integration {
            Integration::HomeAssistant { entity_id } if d.kind == DeviceKind::Light => {
                Some(Entry {
                    name: d.name.clone(),
                    id: entity_id.clone(),
                    state: None,
                })
            }
            _ => None,
        })
        .collect())
}
fn perform(room: &Id, operation: Operation) -> Result<Answer, String> {
    let mut entries = configured(room)?;
    if matches!(operation, Operation::List) && entries.is_empty() {
        return Ok(Answer::List(entries));
    }
    let client = Settings::load(&home::path("ha-connection.json"))
        .and_then(|s| s.client())
        .map_err(|_| "Set up Home Assistant under Connections in the web editor")?;
    match operation {
        Operation::List => {
            let states = client.lights().map_err(|e| e.to_string())?;
            for entry in &mut entries {
                entry.state = states.iter().find(|s| s.entity_id == entry.id).cloned();
            }
            Ok(Answer::List(entries))
        }
        Operation::State(id) | Operation::Send(id, _) if !entries.iter().any(|e| e.id == id) => {
            Err("This light was removed from the room".into())
        }
        Operation::State(id) => client
            .light(&id)
            .map(|s| Answer::State(s, false))
            .map_err(|e| e.to_string()),
        Operation::Send(id, command) => {
            client.command(&id, command).map_err(|e| e.to_string())?;
            client
                .light(&id)
                .map(|s| Answer::State(s, true))
                .map_err(|e| e.to_string())
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
            while let Ok((generation, room, operation)) = requests.recv() {
                let _ = events.send((generation, perform(&room, operation)));
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
        self.buttons(app, "Connecting", "Waiting for Home Assistant…", &["Back"]);
    }
    fn list(&mut self, app: &App) {
        self.page = Page::List;
        let mut items: Vec<_> = self
            .entries
            .iter()
            .map(|e| {
                (
                    e.name.clone(),
                    e.state
                        .as_ref()
                        .map(description)
                        .unwrap_or_else(|| "Missing from Home Assistant".into()),
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
                "Add Home Assistant lights to this room in the web editor."
            } else {
                "Choose a light to control it."
            },
            items,
        );
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
                    self.room = Some(room);
                    self.request(app, Operation::List);
                }
                Input::Back => match self.page {
                    Page::Light | Page::Brightness => self.list(app),
                    _ => self.close(app),
                },
                Input::Pick(i) => match self.page {
                    Page::List => {
                        if let Some(entry) = self.entries.get(i) {
                            self.request(app, Operation::State(entry.id.clone()));
                        } else if i == self.entries.len() {
                            self.request(app, Operation::List);
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
                                self.request(app, Operation::State(light.entity_id));
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
                                self.request(app, Operation::State(light.entity_id))
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
                    self.entries = entries;
                    self.list(app);
                }
                Ok(Answer::State(state, sent)) => {
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
