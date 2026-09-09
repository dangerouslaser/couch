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
    Play(bool),
    Retry,
}
fn command(name: &str) -> Option<Command> {
    Some(match name {
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
struct Work {
    generation: u64,
    action: Command,
    at: Instant,
}
struct Event {
    generation: u64,
    status: Result<String, String>,
}
fn worker(rx: mpsc::Receiver<Work>, tx: mpsc::SyncSender<Event>, active: Arc<AtomicU64>) {
    let mut client = None;
    let mut generation = 0;
    let mut refreshed = Instant::now();
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
        }
        if let Some(w) = work {
            if w.generation != current
                || current == 0
                || (!matches!(w.action, Command::Retry)
                    && w.at.elapsed() > Duration::from_millis(750))
            {
                continue;
            }
            let result = (|| {
                if matches!(w.action, Command::Retry) || client.is_none() {
                    let settings = Settings::load(&home::path("webos-connection.json"))?;
                    let mut connected = Client::connect(&settings)?;
                    connected.prepare_input()?;
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
                    Command::Volume(_) | Command::Mute(_) | Command::Retry
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
        app.on_open_tv(move |name| q.borrow_mut().push(format!("open:{name}")));
        let q = input.clone();
        app.on_tv_action(move |action| q.borrow_mut().push(action.to_string()));
        let (tx, requests) = mpsc::sync_channel(8);
        let (events, rx) = mpsc::sync_channel(16);
        let active = Arc::new(AtomicU64::new(0));
        let current = active.clone();
        std::thread::spawn(move || worker(requests, events, current));
        Self {
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
            let action = if let Some(name) = action.strip_prefix("open:") {
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
                        generation: self.generation,
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
