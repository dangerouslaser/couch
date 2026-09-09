//! LG TV control: one background owner, bounded input, no network on the UI thread.
use crate::{home, App};
use couch_webos::{Button, Client, Playback, Settings};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
enum Command {
    Key(Button),
    Volume(bool),
    Channel(bool),
    Mute(bool),
    ToggleMute,
    Power,
    Play(bool),
    Retry,
}
fn command(name: &str) -> Option<Command> {
    Some(match name {
        "power" => Command::Power,
        "toggle-mute" => Command::ToggleMute,
        "red" => Command::Key(Button::Red),
        "green" => Command::Key(Button::Green),
        "blue" => Command::Key(Button::Blue),
        "yellow" => Command::Key(Button::Yellow),
        "up" => Command::Key(Button::Up),
        "down" => Command::Key(Button::Down),
        "left" => Command::Key(Button::Left),
        "right" => Command::Key(Button::Right),
        "ok" => Command::Key(Button::Enter),
        "back" => Command::Key(Button::Back),
        "home" => Command::Key(Button::Home),
        "menu" => Command::Key(Button::Menu),
        "volume-up" => Command::Volume(true),
        "volume-down" => Command::Volume(false),
        "channel-up" => Command::Channel(true),
        "channel-down" => Command::Channel(false),
        "mute" => Command::Mute(true),
        "unmute" => Command::Mute(false),
        "play" => Command::Play(true),
        "pause" => Command::Play(false),
        "retry" => Command::Retry,
        _ => return None,
    })
}
fn execute(c: &mut Client, action: Command) -> couch_webos::Result<()> {
    match action {
        Command::Key(key) => c.button(key),
        Command::Volume(true) => c.volume_up(),
        Command::Volume(false) => c.volume_down(),
        Command::Channel(up) => c.channel(up),
        Command::Mute(on) => c.mute(on),
        Command::Power => c.power_off(),
        Command::ToggleMute => {
            let status = c.volume()?;
            let status = if status["volumeStatus"].is_object() {
                &status["volumeStatus"]
            } else {
                &status
            };
            let muted = status["muteStatus"]
                .as_bool()
                .or(status["muted"].as_bool())
                .ok_or(couch_webos::Error::Protocol)?;
            c.mute(!muted)
        }
        Command::Play(play) => c.playback(if play {
            Playback::Play
        } else {
            Playback::Pause
        }),
        Command::Retry => Ok(()),
    }
}
fn volume(c: &mut Client) -> couch_webos::Result<String> {
    let v = c.volume()?;
    let v = if v["volumeStatus"].is_object() {
        &v["volumeStatus"]
    } else {
        &v
    };
    Ok(format!(
        "Volume {}{}",
        v["volume"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "—".into()),
        if v["muteStatus"] == true || v["muted"] == true {
            " · Muted"
        } else {
            ""
        }
    ))
}
// Bind the learned wake address to this exact pairing endpoint. It is private
// device state, not part of exported room configuration.
fn remember_wake(settings: &Settings, credentials:&std::path::Path) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let Ok(address) = settings.address() else {
        return;
    };
    let Ok(arp) = std::fs::read_to_string("/proc/net/arp") else {
        return;
    };
    let Some(mac) = arp
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            (fields.len() >= 6 && fields[0] == address.to_string() && fields[2] == "0x2")
                .then(|| fields[3].to_string())
        })
        .next()
    else {
        return;
    };
    if couch_webos::magic_packet(&mac).is_err() || mac == "00:00:00:00:00:00" {
        return;
    }
    let data = serde_json::json!({"url":settings.url,"mac":mac}).to_string();
    let file = credentials.with_file_name("webos-wake.json");
    if std::fs::read_to_string(&file).ok().as_deref() == Some(&data) {
        return;
    }
    let tmp = file.with_extension("new");
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &file)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
}
fn wake_tv(settings: &Settings, credentials:&std::path::Path) -> Result<(), String> {
    let saved = std::fs::read(credentials.with_file_name("webos-wake.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .ok_or("Connect while the TV is on once to learn its wake address")?;
    if saved["url"].as_str() != Some(settings.url.as_str()) {
        return Err("Connect to this TV while it is on once to learn its wake address".into());
    }
    couch_webos::wake(
        saved["mac"].as_str().ok_or("Missing TV wake address")?,
        std::net::Ipv4Addr::BROADCAST,
    )
    .map_err(|e| e.to_string())
}
fn power(
    client: &mut Option<Client>,
    active: &AtomicU64,
    generation: u64,
    credentials:&std::path::Path,
) -> Result<String, String> {
    let settings =
        Settings::load(credentials).map_err(|e| e.to_string())?;
    if client.is_none() {
        match Client::connect(&settings) {
            Ok(c) => *client = Some(c),
            Err(couch_webos::Error::Transport | couch_webos::Error::Timeout) => {
                if active.load(Ordering::SeqCst) != generation {
                    return Ok(String::new());
                }
                wake_tv(&settings,credentials)?;
                return Ok("Wake requested…".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    if active.load(Ordering::SeqCst) != generation {
        return Ok(String::new());
    }
    remember_wake(&settings,credentials);
    let status = client
        .as_mut()
        .unwrap()
        .power_state()
        .map_err(|e| e.to_string())?;
    let state = status["state"]
        .as_str()
        .ok_or("TV did not report its power state")?;
    if active.load(Ordering::SeqCst) != generation {
        return Ok(String::new());
    }
    if state != "Active" {
        wake_tv(&settings,credentials)?;
        *client = None;
        return Ok("Wake requested…".into());
    }
    // A failed power-off write is ambiguous: never follow it with a wake packet.
    client
        .as_mut()
        .unwrap()
        .power_off()
        .map_err(|e| e.to_string())?;
    *client = None;
    Ok("TV powered off · Press Power to wake".into())
}
struct Work {
    connection:String,
    generation: u64,
    action: Command,
    at: Instant,
}
struct Event {
    generation: u64,
    status: Result<String, String>,
}
fn worker(rx: mpsc::Receiver<Work>, tx: mpsc::SyncSender<Event>, active: Arc<AtomicU64>) {
    let mut credentials=home::path("webos-connection.json");
    let mut client = None;
    let mut generation = 0;
    let mut refreshed = Instant::now();
    let mut waking: Option<Instant> = None;
    loop {
        let work = match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(w) => Some(w),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(_) => return,
        };
        let current = active.load(Ordering::SeqCst);
        if current != generation {
            client = None;
            generation = current;
            waking = None;
        }
        if let Some(w) = work {
            if w.generation != current
                || current == 0
                || (!matches!(w.action, Command::Retry)
                    && w.at.elapsed() > Duration::from_millis(750))
            {
                continue;
            }
            credentials=crate::connections::file(&w.connection,"webos");
            if !w.connection.is_empty() && !crate::connections::config().is_some_and(|c|c.connection(&couch_model::Id::new(&w.connection)).is_some_and(|c|c.provider==couch_model::Provider::WebOs)) {continue;}
            if matches!(w.action, Command::Power) {
                waking = None;
                let result = power(&mut client, &active, generation,&credentials);
                if result.as_deref() == Ok("Wake requested…") {
                    waking = Some(Instant::now() + Duration::from_secs(30));
                }
                if result.is_err() {
                    client = None;
                }
                let _ = tx.try_send(Event {
                    generation,
                    status: result,
                });
                refreshed = Instant::now();
                continue;
            }
            let result = (|| {
                if matches!(w.action, Command::Retry) || client.is_none() {
                    let settings = Settings::load(&credentials)?;
                    let mut connected = Client::connect(&settings)?;
                    connected.prepare_input()?;
                    remember_wake(&settings,&credentials);
                    client = Some(connected);
                }
                // Opening or closing another screen cancels queued keys, including
                // one that was waiting for connection establishment.
                if active.load(Ordering::SeqCst) != generation
                    || (!matches!(w.action, Command::Retry)
                        && w.at.elapsed() > Duration::from_millis(750))
                {
                    return Ok(String::new());
                }
                let c = client.as_mut().unwrap();
                execute(c, w.action)?;
                if matches!(
                    w.action,
                    Command::Volume(_) | Command::Mute(_) | Command::ToggleMute | Command::Retry
                ) {
                    volume(c)
                } else {
                    Ok(String::new())
                }
            })();
            if let Err(ref error) = result {
                if !matches!(error, couch_webos::Error::Rejected) {
                    client = None;
                }
            }
            let _ = tx.try_send(Event {
                generation,
                status: result.map_err(|e| e.to_string()),
            });
            refreshed = Instant::now();
        } else if current != 0 && waking.is_some() && refreshed.elapsed() > Duration::from_secs(2) {
            let connected = Settings::load(&credentials)
                .and_then(|s| Client::connect(&s))
                .and_then(|mut c| {
                    if c.power_state()?["state"] == "Active" {
                        Ok(c)
                    } else {
                        Err(couch_webos::Error::Timeout)
                    }
                });
            if let Ok(mut c) = connected {
                let result = volume(&mut c).map_err(|e| e.to_string());
                client = Some(c);
                waking = None;
                let _ = tx.try_send(Event {
                    generation,
                    status: result,
                });
            } else if waking.is_some_and(|until| Instant::now() >= until) {
                waking = None;
                let _ = tx.try_send(Event {
                    generation,
                    status: Err(
                        "TV did not wake. Enable network/mobile power-on in the LG TV settings"
                            .into(),
                    ),
                });
            }
            refreshed = Instant::now();
        } else if current != 0 && refreshed.elapsed() > Duration::from_secs(5) {
            if let Some(c) = client.as_mut() {
                let result = volume(c);
                if result.is_err() {
                    client = None;
                }
                let _ = tx.try_send(Event {
                    generation,
                    status: result.map_err(|e| e.to_string()),
                });
            }
            refreshed = Instant::now();
        }
    }
}
pub struct Controller {
    connection:String,
    input: Rc<RefCell<Vec<String>>>,
    tx: mpsc::SyncSender<Work>,
    rx: mpsc::Receiver<Event>,
    active: Arc<AtomicU64>,
    generation: u64,
}
impl Controller {
    pub fn new(app: &App) -> Self {
        let input = Rc::new(RefCell::new(Vec::new()));
        let q = input.clone();
        app.on_open_tv(move |id,name| q.borrow_mut().push(format!("open:{id}/{name}")));
        let q = input.clone();
        app.on_tv_action(move |action| q.borrow_mut().push(action.to_string()));
        let (tx, requests) = mpsc::sync_channel(8);
        let (events, rx) = mpsc::sync_channel(16);
        let active = Arc::new(AtomicU64::new(0));
        let current = active.clone();
        std::thread::spawn(move || worker(requests, events, current));
        Self {
            connection:String::new(),
            input,
            tx,
            rx,
            active,
            generation: 0,
        }
    }
    pub fn poll(&mut self, app: &App) {
        let inputs = std::mem::take(&mut *self.input.borrow_mut());
        for action in inputs {
            let action = if let Some(target) = action.strip_prefix("open:") {
                let (connection,name)=target.split_once('/').unwrap_or(("",target));
                self.connection=connection.into();
                self.generation += 1;
                self.active.store(self.generation, Ordering::SeqCst);
                app.set_tv_title(name.into());
                app.set_tv_status("Connecting to TV…".into());
                app.set_tv_error("".into());
                app.set_tv_shown(true);
                app.invoke_focus_tv();
                "retry"
            } else {
                action.as_str()
            };
            if action == "close" {
                self.active.store(0, Ordering::SeqCst);
                app.set_tv_shown(false);
                app.set_tv_error("".into());
                if app.get_light_shown() {
                    app.invoke_focus_light();
                } else {
                    app.invoke_focus_home();
                }
                continue;
            }
            if !app.get_tv_shown() {
                continue;
            }
            if let Some(action) = command(action) {
                if self
                    .tx
                    .try_send(Work {
                        connection:self.connection.clone(),                        generation: self.generation,
                        action,
                        at: Instant::now(),
                    })
                    .is_err()
                {
                    app.set_tv_error("TV is busy; try again.".into());
                }
            }
        }
        while let Ok(event) = self.rx.try_recv() {
            if event.generation != self.active.load(Ordering::SeqCst) || !app.get_tv_shown() {
                continue;
            }
            match event.status {
                Ok(status) => {
                    app.set_tv_error("".into());
                    if !status.is_empty() {
                        app.set_tv_status(status.into());
                    }
                }
                Err(error) => {
                    app.set_tv_status("Connection needs attention".into());
                    app.set_tv_error(
                        format!("{error}. Check the TV is on, then reconnect.").into(),
                    );
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_actions_are_explicit_and_back_belongs_to_tv() {
        assert!(matches!(command("back"), Some(Command::Key(Button::Back))));
        assert!(matches!(command("ok"), Some(Command::Key(Button::Enter))));
        assert!(matches!(
            command("channel-down"),
            Some(Command::Channel(false))
        ));
        assert!(command("close").is_none());
        assert!(command("unknown").is_none());
    }
}
