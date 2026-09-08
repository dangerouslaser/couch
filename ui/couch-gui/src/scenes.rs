//! Scene recall and room-scoped channel navigation; network work stays off the GUI thread.
use crate::{home, App};
use couch_model::{Config, Id, Provider};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

enum Input {
    Recall(Id),
    Cycle(Id, i32),
}
pub struct Controller {
    input: Rc<RefCell<VecDeque<Input>>>,
    tx: mpsc::SyncSender<(u64, Id)>,
    rx: mpsc::Receiver<(u64, Result<(), String>)>,
    pending: Option<(u64, Id)>,
    busy: bool,
    sequence: u64,
    cursor: HashMap<Id, Id>,
    until: Option<Instant>,
}
fn config() -> Result<Config, String> {
    serde_json::from_slice(
        &std::fs::read(home::path("config.json")).map_err(|_| "Cannot read scenes")?,
    )
    .map_err(|_| "Cannot read scenes".into())
}
fn next_scene(ids: &[Id], current: Option<&Id>, delta: i32) -> Option<Id> {
    if ids.is_empty() {
        return None;
    }
    let index = match current.and_then(|id| ids.iter().position(|s| s == id)) {
        Some(i) => (i as i32 + delta.signum()).rem_euclid(ids.len() as i32) as usize,
        None if delta < 0 => ids.len() - 1,
        None => 0,
    };
    Some(ids[index].clone())
}
impl Controller {
    pub fn new(app: &App) -> Self {
        let input = Rc::new(RefCell::new(VecDeque::new()));
        let q = input.clone();
        let weak = app.as_weak();
        app.on_room_scene_step(move |delta| {
            if let Some(app) = weak.upgrade() {
                q.borrow_mut().push_back(Input::Cycle(
                    Id::new(app.get_light_room_id().as_str()),
                    delta,
                ));
            }
        });
        let (tx, requests) = mpsc::sync_channel::<(u64, Id)>(1);
        let (events, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok((sequence, id)) = requests.recv() {
                let result = (|| -> Result<(), String> {
                    let cfg = config()?;
                    let scene = cfg.scene(&id).ok_or("Scene was removed")?;
                    let hue = scene
                        .hue
                        .as_ref()
                        .ok_or("Device-step scenes are not supported yet")?;
                    if !cfg
                        .connection(&hue.connection_id)
                        .is_some_and(|c| c.provider == Provider::Hue)
                    {
                        return Err("Hue connection was removed".into());
                    }
                    couch_hue::settings::Settings::load(&home::path("hue-connection.json"))
                        .and_then(|s| s.client())
                        .and_then(|c| c.recall_scene(&hue.scene_id))
                        .map_err(|e| e.to_string())
                })();
                let _ = events.send((sequence, result));
            }
        });
        Self {
            input,
            tx,
            rx,
            pending: None,
            busy: false,
            sequence: 0,
            cursor: HashMap::new(),
            until: None,
        }
    }
    pub fn opener(&self) -> impl Fn(Id) + 'static {
        let input = self.input.clone();
        move |id| input.borrow_mut().push_back(Input::Recall(id))
    }
    fn feedback(&mut self, app: &App, name: &str, status: &str) {
        app.set_scene_feedback_name(name.into());
        app.set_scene_feedback_status(status.into());
        app.set_scene_feedback_shown(true);
        app.set_brightness_shown(false);
        self.until = Some(Instant::now() + Duration::from_secs(3));
    }
    pub fn poll(&mut self, app: &App) {
        if self.until.is_some_and(|until| Instant::now() >= until) {
            app.set_scene_feedback_shown(false);
            self.until = None;
        }
        loop {
            let Some(input) = self.input.borrow_mut().pop_front() else {
                break;
            };
            let cfg = match config() {
                Ok(cfg) => cfg,
                Err(error) => {
                    self.feedback(app, &error, "Scene unavailable");
                    continue;
                }
            };
            let id = match input {
                Input::Recall(id) => Some(id),
                Input::Cycle(room, delta) => {
                    let ids: Vec<_> = cfg
                        .scenes
                        .iter()
                        .filter(|s| s.rooms.contains(&room))
                        .map(|s| s.id.clone())
                        .collect();
                    let id = next_scene(&ids, self.cursor.get(&room), delta);
                    if id.is_none() {
                        self.feedback(app, "Add scenes in the web UI", "No scenes in this room");
                    }
                    id
                }
            };
            let Some(id) = id else { continue };
            let Some(scene) = cfg.scene(&id) else {
                self.feedback(app, "Scene was removed", "Scene unavailable");
                continue;
            };
            for room in &scene.rooms {
                self.cursor.insert(room.clone(), id.clone());
            }
            self.sequence += 1;
            // Keep only the latest unsent choice while a previous recall finishes.
            self.pending = Some((self.sequence, id));
            self.feedback(app, &scene.name, "Applying scene…");
        }
        while let Ok((sequence, result)) = self.rx.try_recv() {
            self.busy = false;
            // An older response must not replace feedback for a newer selection.
            if sequence == self.sequence {
                app.set_scene_feedback_shown(true);
                match result {
                    Ok(()) => app.set_scene_feedback_status("Scene activated".into()),
                    Err(error) => {
                        app.set_scene_feedback_status("Scene failed".into());
                        app.set_scene_feedback_name(error.into());
                    }
                }
                self.until = Some(Instant::now() + Duration::from_secs(3));
            }
        }
        if !self.busy {
            if let Some(request) = self.pending.take() {
                if self.tx.try_send(request).is_ok() {
                    self.busy = true;
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cycling_wraps_and_handles_empty_or_removed_selections() {
        let ids = vec![Id::new("relax"), Id::new("bright"), Id::new("night")];
        assert_eq!(next_scene(&ids, None, 1), Some(ids[0].clone()));
        assert_eq!(next_scene(&ids, None, -1), Some(ids[2].clone()));
        assert_eq!(next_scene(&ids, Some(&ids[2]), 1), Some(ids[0].clone()));
        assert_eq!(next_scene(&ids, Some(&ids[0]), -1), Some(ids[2].clone()));
        assert_eq!(
            next_scene(&ids, Some(&Id::new("removed")), 1),
            Some(ids[0].clone())
        );
        assert_eq!(next_scene(&[], None, 1), None);
        assert_eq!(
            next_scene(&ids[..1], Some(&ids[0]), 1),
            Some(ids[0].clone())
        );
    }
}
