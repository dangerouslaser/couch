//! Scene recall worker shared by the home and room scene launchers.
use crate::home;
use couch_model::{Config, Id, Provider};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
pub struct Controller {
    tx: mpsc::SyncSender<Id>,
    rx: mpsc::Receiver<String>,
    busy: Arc<AtomicBool>,
}
impl Controller {
    pub fn new() -> Self {
        let (tx, requests) = mpsc::sync_channel::<Id>(1);
        let (events, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(id) = requests.recv() {
                let result = (|| -> Result<String, String> {
                    let cfg: Config = serde_json::from_slice(
                        &std::fs::read(home::path("config.json"))
                            .map_err(|_| "Cannot read scenes")?,
                    )
                    .map_err(|_| "Cannot read scenes")?;
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
                        .map_err(|e| e.to_string())?;
                    Ok(format!("{} activated", scene.name))
                })();
                let _ = events.send(result.unwrap_or_else(|e| e));
            }
        });
        Self {
            tx,
            rx,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn opener(&self) -> impl Fn(Id) + 'static {
        let tx = self.tx.clone();
        let busy = self.busy.clone();
        move |id| {
            if !busy.swap(true, Ordering::SeqCst) && tx.try_send(id).is_err() {
                busy.store(false, Ordering::SeqCst);
            }
        }
    }
    pub fn poll(&self) -> Option<String> {
        let result = self.rx.try_recv().ok()?;
        self.busy.store(false, Ordering::SeqCst);
        Some(result)
    }
}
