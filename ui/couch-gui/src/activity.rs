//! Cinema activity controller. All Kodi I/O and image decoding stay off Slint.
use crate::{home, App, PlayerChoice};
use couch_kodi::{
    playback::{Chapter, Playback},
    Kodi,
};
use couch_model::{Config, Integration};
use serde_json::{json, Value};
use slint::{ModelRc, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Target {
    connection: String,
    host: String,
    port: u16,
    name: String,
    room: String,
}
fn target(config: &Config, id: &str) -> Result<Target, String> {
    let (device, name, room) = if let Some(id) = id.strip_prefix("device:") {
        let room = config
            .rooms
            .iter()
            .find(|r| r.devices.iter().any(|d| d.id.as_str() == id))
            .ok_or("Device was removed")?;
        let device = room.devices.iter().find(|d| d.id.as_str() == id).unwrap();
        (device, device.name.clone(), room.name.clone())
    } else {
        let activity = config
            .activities
            .iter()
            .find(|a| a.id.as_str() == id)
            .ok_or("Activity was removed")?;
        let room = config.room(&activity.room).ok_or("Room was removed")?;
        let source = activity
            .source
            .as_ref()
            .ok_or("Choose a Kodi source for this activity in the web UI")?;
        let device = config
            .devices()
            .find(|(_, d)| &d.id == source)
            .map(|(_, d)| d)
            .ok_or("Source was removed")?;
        (device, activity.name.clone(), room.name.clone())
    };
    match config.resolve_integration(&device.integration) {
        Some(Integration::Kodi { host, port }) => Ok(Target {
            connection: match &device.integration {
                Integration::Connection { connection_id, .. } => connection_id.to_string(),
                _ => String::new(),
            },
            host,
            port,
            name,
            room,
        }),
        _ => Err("This activity needs a Kodi source device".into()),
    }
}
fn kodi_client(t: &Target) -> Kodi {
    if let Ok(s) =
        couch_kodi::settings::Settings::load(&crate::connections::file(&t.connection, "kodi"))
    {
        if s.host == t.host && s.http_control {
            return s.client();
        }
    }
    Kodi::tcp(&t.host, t.port)
}
fn seconds(v: &Value) -> f64 {
    ["hours", "minutes", "seconds", "milliseconds"]
        .iter()
        .zip([3600., 60., 1., 0.001])
        .map(|(k, s)| v[k].as_f64().unwrap_or(0.) * s)
        .sum()
}
fn clock(t: f64) -> String {
    let t = t.max(0.) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}
fn identity(p: &Playback) -> String {
    format!("{}:{}:{}", p.player, p.item["file"], p.item["id"])
}
fn title(p: &Playback) -> String {
    p.item["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(p.item["label"].as_str())
        .unwrap_or("Now playing")
        .into()
}
fn art(p: &Playback, key: &str) -> String {
    p.item["art"][key]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| p.item["art"][format!("tvshow.{key}")].as_str())
        .unwrap_or("")
        .into()
}
enum Request {
    Open(Target),
    Close,
    Command(String, Value, String),
}
struct Snapshot {
    playing: Option<Playback>,
    chapters: Option<Vec<Chapter>>,
}
enum Event {
    State(Result<Snapshot, String>),
    Done(Result<(), String>),
}
fn worker(rx: mpsc::Receiver<(u64, Request)>, tx: mpsc::SyncSender<(u64, Event)>) {
    let mut client: Option<Kodi> = None;
    let mut generation = 0;
    let mut last = Instant::now() - Duration::from_secs(10);
    let mut known = String::new();
    let mut chapters = None;
    let mut current: Option<Playback> = None;
    let mut connected = false;
    loop {
        let mut completed = None;
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok((g, Request::Open(t))) => {
                generation = g;
                client = Some(kodi_client(&t).with_timeout(Duration::from_secs(2)));
                known.clear();
                chapters = None;
                current = None;
                connected = false;
                last = Instant::now() - Duration::from_secs(10);
            }
            Ok((_, Request::Close)) => {
                client = None;
                current = None;
            }
            Ok((g, Request::Command(method, params, item))) if g == generation => {
                let result = (|| {
                    let c = client.as_ref().ok_or("Kodi is unavailable")?;
                    if method.starts_with("Input.") || method == "Application.SetMute" {
                        c.call(&method, params)
                            .map_err(|_| "Kodi rejected that input")?;
                    } else if method == "Application.SetVolume" {
                        let delta = params["delta"].as_i64().unwrap_or(0);
                        let volume = c.volume().map_err(|_| "Cannot read Kodi volume")?;
                        c.set_volume((volume.volume + delta).clamp(0, 100))
                            .map_err(|_| "Cannot change Kodi volume")?;
                    } else {
                        let p = current
                            .as_ref()
                            .filter(|p| identity(p) == item)
                            .ok_or("Playback changed; try again")?;
                        c.player_command(p.player, &method, params)
                            .map_err(|_| "Kodi rejected that command")?;
                    }
                    Ok(())
                })()
                .map_err(str::to_string);
                completed = Some(result);
                // Refresh once after acknowledgement; never publish a pre-command snapshot afterwards.
                last = Instant::now() - Duration::from_secs(10);
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(_) => {}
        }
        let Some(c) = client.as_ref() else { continue };
        let changed = connected
            && c.next_notification(Duration::from_millis(1))
                .ok()
                .flatten()
                .is_some();
        if changed || last.elapsed() >= Duration::from_secs(5) {
            last = Instant::now();
            let result = c
                .playback()
                .map(|p| {
                    let key = p.as_ref().map(identity).unwrap_or_default();
                    if key != known {
                        chapters = p.as_ref().and_then(|p| c.chapters(p.player).ok().flatten());
                        known = key;
                    }
                    current = p.clone();
                    Snapshot {
                        playing: p,
                        chapters: chapters.clone(),
                    }
                })
                .map_err(|_| {
                    current = None;
                    "Kodi is unavailable. Check the player and its remote-control settings."
                        .to_string()
                });
            connected = result.is_ok();
            if tx.send((generation, Event::State(result))).is_err() {
                return;
            }
        }
        if let Some(result) = completed {
            if tx.send((generation, Event::Done(result))).is_err() {
                return;
            }
        }
    }
}

pub struct Controller {
    input: Rc<RefCell<Vec<(String, f64)>>>,
    tx: mpsc::SyncSender<(u64, Request)>,
    rx: mpsc::Receiver<(u64, Event)>,
    generation: u64,
    snapshot: Option<Snapshot>,
    at: Instant,
    tick: Instant,
    target: Option<Target>,
    busy: bool,
    error_until: Option<Instant>,
    artwork: super::activity_art::Worker,
    art_key: String,
}
impl Controller {
    pub fn new(app: &App) -> Self {
        let input = Rc::new(RefCell::new(Vec::new()));
        let queue = input.clone();
        app.on_open_activity(move |id| queue.borrow_mut().push((format!("open:{id}"), 0.)));
        let queue = input.clone();
        app.on_player_action(move |action, value| {
            queue.borrow_mut().push((action.into(), value as f64))
        });
        let (tx, rx) = mpsc::sync_channel(16);
        let (events, receive) = mpsc::sync_channel(4);
        std::thread::spawn(move || worker(rx, events));
        Self {
            input,
            tx,
            rx: receive,
            generation: 0,
            snapshot: None,
            at: Instant::now(),
            tick: Instant::now(),
            target: None,
            busy: false,
            error_until: None,
            artwork: super::activity_art::Worker::new(),
            art_key: String::new(),
        }
    }
    fn error(&mut self, app: &App, text: &str) {
        app.set_player_message(text.into());
        self.error_until = Some(Instant::now() + Duration::from_secs(4));
    }
    fn open(&mut self, app: &App, id: &str) {
        if let Some(config) = std::fs::read(home::path("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
        {
            let source = config
                .activities
                .iter()
                .find(|a| a.id.as_str() == id)
                .and_then(|a| a.source.as_ref());
            if let Some((_, device)) = config.devices().find(|(_, d)| Some(&d.id) == source) {
                if matches!(
                    config.resolve_integration(&device.integration),
                    Some(Integration::WebOs)
                ) {
                    let connection = match &device.integration {
                        Integration::Connection { connection_id, .. } => connection_id.to_string(),
                        _ => config
                            .connections
                            .iter()
                            .find(|c| c.provider == couch_model::Provider::WebOs)
                            .map(|c| c.id.to_string())
                            .unwrap_or_default(),
                    };
                    app.invoke_open_tv(connection.as_str().into(), device.name.as_str().into());
                    return;
                }
            }
        }
        self.generation += 1;
        self.busy = false;
        self.snapshot = None;
        self.art_key.clear();
        app.set_player_shown(true);
        app.set_player_panel(0);
        app.set_player_ready(false);
        app.set_player_connected(false);
        app.set_player_message("".into());
        app.set_player_title("Connecting to Kodi…".into());
        app.set_player_fanart(slint::Image::default());
        app.set_player_logo(slint::Image::default());
        app.set_player_has_logo(false);
        app.set_player_has_art(false);
        app.set_player_activity("Watch Kodi".into());
        app.set_player_room("".into());
        app.invoke_focus_player();
        let result = std::fs::read(home::path("config.json"))
            .map_err(|_| "Cannot read configuration".into())
            .and_then(|b| {
                serde_json::from_slice::<Config>(&b).map_err(|_| "Cannot read configuration".into())
            })
            .and_then(|c| target(&c, id));
        match result {
            Ok(t) => {
                app.set_player_activity(t.name.clone().into());
                app.set_player_room(t.room.clone().into());
                if self
                    .tx
                    .try_send((self.generation, Request::Open(t.clone())))
                    .is_err()
                {
                    self.error(app, "Connection busy. Reopen the activity.");
                }
                self.target = Some(t);
            }
            Err(e) => {
                app.set_player_title(e.into());
                self.target = None;
                let _ = self.tx.try_send((self.generation, Request::Close));
            }
        }
    }
    fn send(&mut self, app: &App, method: &str, params: Value) {
        if self.busy {
            return;
        }
        let item = self
            .snapshot
            .as_ref()
            .and_then(|s| s.playing.as_ref())
            .map(identity)
            .unwrap_or_default();
        if self
            .tx
            .try_send((
                self.generation,
                Request::Command(method.into(), params, item),
            ))
            .is_ok()
        {
            self.busy = true
        } else {
            self.error(app, "Connection busy. Try again.");
        }
    }
    fn choose(&mut self, app: &App, index: usize) {
        let Some(s) = &self.snapshot else { return };
        let Some(p) = &s.playing else { return };
        let panel = app.get_player_panel();
        let command=match panel {
            1=>s.chapters.as_ref().and_then(|c|c.get(index)).map(|c|("Player.Seek",json!({"value":{"time":{"hours":c.time/3600,"minutes":c.time/60%60,"seconds":c.time%60,"milliseconds":0}}}))),
            2=>p.properties["audiostreams"].as_array().and_then(|v|v.get(index)).map(|v|("Player.SetAudioStream",json!({"stream":v["index"]}))),
            3=>if index==0 {Some(("Player.SetSubtitle",json!({"subtitle":"off"})))} else {p.properties["subtitles"].as_array().and_then(|v|v.get(index-1)).map(|v|("Player.SetSubtitle",json!({"subtitle":v["index"],"enable":true})))},
            _=>None,
        };
        if let Some((method, params)) = command {
            self.send(app, method, params);
            app.set_player_panel(0);
            app.invoke_focus_player();
        }
    }
    fn panel(&mut self, app: &App, panel: i32) {
        let mut rows = Vec::new();
        let mut detail = String::new();
        if let Some(s) = &self.snapshot {
            if let Some(p) = &s.playing {
                if panel == 1 {
                    match &s.chapters {
                Some(chapters) if !chapters.is_empty()=>for c in chapters {rows.push(PlayerChoice {title:if c.name.is_empty(){format!("Chapter {}",c.index)}else{c.name.clone()}.into(),detail:clock(c.time as f64).into()});},
                _=>detail="No chapter list is available from this Kodi player. Use the progress bar to seek.".into(),
            }
                } else {
                    if panel == 3 {
                        rows.push(PlayerChoice {
                            title: "Off".into(),
                            detail: "".into(),
                        });
                    }
                    let field = if panel == 2 {
                        "audiostreams"
                    } else {
                        "subtitles"
                    };
                    if let Some(streams) = p.properties[field].as_array() {
                        for v in streams {
                            let label = v["name"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .or(v["language"].as_str())
                                .unwrap_or("Track");
                            rows.push(PlayerChoice {
                                title: label.into(),
                                detail: v["codec"].as_str().unwrap_or("").into(),
                            });
                        }
                    }
                    if rows.is_empty() {
                        detail = "No tracks available".into()
                    }
                }
            }
        }
        app.set_player_choices(ModelRc::new(VecModel::from(rows)));
        app.set_player_panel_detail(detail.into());
        app.set_player_panel(panel);
        app.invoke_focus_player();
    }
    pub fn navigation_pending(&self, app: &App) -> bool {
        self.input.borrow().iter().any(|(action, _)| {
            action.starts_with("open:") || action == "back" && app.get_player_panel() == 0
        })
    }
    pub fn poll(&mut self, app: &App) {
        let inputs = std::mem::take(&mut *self.input.borrow_mut());
        for (action, value) in inputs {
            if let Some(id) = action.strip_prefix("open:") {
                self.open(app, id);
                continue;
            }
            if !app.get_player_shown() {
                continue;
            }
            match action.as_str() {
                "back" => {
                    if app.get_player_panel() != 0 {
                        app.set_player_panel(0);
                        app.invoke_focus_player();
                    } else {
                        self.generation += 1;
                        self.busy = false;
                        self.snapshot = None;
                        app.set_player_shown(false);
                        app.set_player_message("".into());
                        let _ = self.tx.try_send((self.generation, Request::Close));
                        if app.get_light_shown() {
                            app.invoke_focus_light()
                        } else {
                            app.invoke_focus_home()
                        }
                    }
                }
                "retry" => {
                    if let Some(t) = self.target.clone() {
                        let _ = self.tx.try_send((self.generation, Request::Open(t)));
                    }
                }
                "play" => {
                    if let Some(p) = self.snapshot.as_ref().and_then(|s| s.playing.as_ref()) {
                        self.send(
                            app,
                            "Player.PlayPause",
                            json!({"play":p.properties["speed"].as_f64().unwrap_or(0.)==0.}),
                        );
                    }
                }
                "seek" if app.get_player_can_seek() => self.send(
                    app,
                    "Player.Seek",
                    json!({"value":{"percentage":value.clamp(0.,100.)}}),
                ),
                "skip" if app.get_player_can_seek() => self.send(
                    app,
                    "Player.Seek",
                    json!({"value":{"seconds":value as i64}}),
                ),
                "mute" => self.send(app, "Application.SetMute", json!({"mute":"toggle"})),
                "volume" => self.send(app, "Application.SetVolume", json!({"delta":value as i64})),
                "chapters" => self.panel(app, 1),
                "audio" => self.panel(app, 2),
                "subtitles" => self.panel(app, 3),
                "choose" => self.choose(app, value as usize),
                "chapter-step" => {
                    if let Some(s) = &self.snapshot {
                        if let (Some(p), Some(chapters)) = (&s.playing, &s.chapters) {
                            let pos = seconds(&p.properties["time"])
                                + self.at.elapsed().as_secs_f64()
                                    * p.properties["speed"].as_f64().unwrap_or(0.);
                            let next = if value > 0. {
                                chapters.iter().find(|c| c.time as f64 > pos + 1.)
                            } else {
                                chapters.iter().rev().find(|c| (c.time as f64) < pos - 3.)
                            };
                            if let Some(c) = next {
                                let t = c.time;
                                self.send(app,"Player.Seek",json!({"value":{"time":{"hours":t/3600,"minutes":t/60%60,"seconds":t%60,"milliseconds":0}}}));
                            }
                        }
                    }
                }
                a if a.starts_with("Input.") => self.send(app, a, json!({})),
                _ => {}
            }
        }
        while let Ok((g, event)) = self.rx.try_recv() {
            if g != self.generation || !app.get_player_shown() {
                continue;
            }
            match event {
                Event::Done(result) => {
                    self.busy = false;
                    if let Err(e) = result {
                        self.error(app, &e)
                    }
                }
                Event::State(result) => match result {
                    Ok(s) => {
                        let previous = self.snapshot.as_ref().and_then(|s| s.playing.as_ref());
                        let changed = previous.map(identity) != s.playing.as_ref().map(identity)
                            || previous.map(|p| {
                                (&p.properties["audiostreams"], &p.properties["subtitles"])
                            }) != s.playing.as_ref().map(|p| {
                                (&p.properties["audiostreams"], &p.properties["subtitles"])
                            });
                        if changed && app.get_player_panel() > 0 && app.get_player_panel() < 4 {
                            app.set_player_panel(0);
                            app.invoke_focus_player();
                        }
                        self.at = Instant::now();
                        app.set_player_ready(s.playing.is_some());
                        app.set_player_connected(true);
                        if let Some(p) = &s.playing {
                            app.set_player_title(title(p).into());
                            app.set_player_paused(
                                p.properties["speed"].as_f64().unwrap_or(0.) == 0.,
                            );
                            app.set_player_can_seek(
                                p.properties["canseek"].as_bool().unwrap_or(false),
                            );
                            let meta = if let Some(show) =
                                p.item["showtitle"].as_str().filter(|s| !s.is_empty())
                            {
                                format!(
                                    "{} · S{} E{}",
                                    show,
                                    p.item["season"].as_i64().unwrap_or(0),
                                    p.item["episode"].as_i64().unwrap_or(0)
                                )
                            } else {
                                p.item["year"]
                                    .as_i64()
                                    .filter(|n| *n > 0)
                                    .map(|y| y.to_string())
                                    .unwrap_or_default()
                            };
                            app.set_player_metadata(meta.into());
                            if let Some(t) = &self.target {
                                let key = format!(
                                    "{}:{}:{}",
                                    identity(p),
                                    art(p, "fanart"),
                                    art(p, "clearlogo")
                                );
                                if key != self.art_key {
                                    app.set_player_has_logo(false);
                                    app.set_player_has_art(false);
                                    if self.artwork.request(
                                        self.generation,
                                        key.clone(),
                                        t.host.clone(),
                                        t.connection.clone(),
                                        art(p, "fanart"),
                                        art(p, "clearlogo"),
                                    ) {
                                        self.art_key = key;
                                    }
                                }
                            }
                        } else {
                            app.set_player_title(
                                "Connected to Kodi.\nUse the remote to choose something on your TV.".into(),
                            );
                            app.set_player_has_logo(false);
                            app.set_player_has_art(false);
                            self.art_key.clear();
                        }
                        self.snapshot = Some(s);
                    }
                    Err(e) => {
                        if app.get_player_panel() > 0 && app.get_player_panel() < 4 {
                            app.set_player_panel(0);
                            app.invoke_focus_player();
                        }
                        self.snapshot = None;
                        app.set_player_ready(false);
                        app.set_player_connected(false);
                        app.set_player_title(e.into());
                        app.set_player_has_logo(false);
                        app.set_player_has_art(false);
                        self.art_key.clear();
                    }
                },
            }
        }
        self.artwork.poll(app, self.generation, &self.art_key);
        if self.tick.elapsed() >= Duration::from_secs(1) {
            self.tick = Instant::now();
            if let Some(p) = self.snapshot.as_ref().and_then(|s| s.playing.as_ref()) {
                let total = seconds(&p.properties["totaltime"]);
                let time = (seconds(&p.properties["time"])
                    + self.at.elapsed().as_secs_f64()
                        * p.properties["speed"].as_f64().unwrap_or(0.))
                .max(0.)
                .min(total.max(0.));
                app.set_player_elapsed(clock(time).into());
                app.set_player_remaining(
                    if total > 0. {
                        format!("−{}", clock(total - time))
                    } else {
                        "LIVE".into()
                    }
                    .into(),
                );
                app.set_player_progress(if total > 0. {
                    (time / total * 100.) as f32
                } else {
                    0.
                });
            }
        }
        if self.error_until.is_some_and(|t| Instant::now() >= t) {
            app.set_player_message("".into());
            self.error_until = None;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_keys_control_kodi_without_opening_an_overlay() {
        // Slint is configured as single-threaded on this target. Run this
        // window test separately from the scene-controller window fixture.
        if std::env::var_os("COUCH_TEST_KODI_KEYS").is_none() {
            let out=std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","activity::tests::physical_keys_control_kodi_without_opening_an_overlay"])
                .env("COUCH_TEST_KODI_KEYS","1").output().unwrap();
            assert!(out.status.success(),"{}\n{}",String::from_utf8_lossy(&out.stdout),String::from_utf8_lossy(&out.stderr));
            return;
        }
        use slint::{ComponentHandle, platform::{Key, WindowEvent}};
        let window=crate::panel::CouchPlatform::install(slint::PhysicalSize::new(480,800)).unwrap();
        let app=App::new().unwrap();
        let actions=Rc::new(RefCell::new(Vec::new()));let received=actions.clone();
        app.on_player_action(move |name,_|received.borrow_mut().push(name.to_string()));
        app.set_player_shown(true);app.set_player_connected(true);app.show().unwrap();app.invoke_focus_player();
        for playing in [false,true] {
            app.set_player_ready(playing);
            for key in [Key::UpArrow,Key::DownArrow,Key::LeftArrow,Key::RightArrow,Key::Return,Key::Escape,Key::Home,Key::F14] {
                let text=char::from(key).to_string().into();
                window.dispatch_event(WindowEvent::KeyPressed{text});
            }
            assert_eq!(&*actions.borrow(), &["Input.Up","Input.Down","Input.Left","Input.Right","Input.Select","Input.Back","Input.Home","mute"]);
            actions.borrow_mut().clear();assert_eq!(app.get_player_panel(),0);
        }
        app.hide().unwrap();
    }
    #[test]
    fn clocks_and_source_validation() {
        assert_eq!(clock(3661.), "1:01:01");
        assert_eq!(clock(-1.), "0:00");
        assert!(target(&Config::default(), "gone").is_err());
        assert_eq!(seconds(&json!({"minutes":2,"seconds":3})), 123.);
    }
}
