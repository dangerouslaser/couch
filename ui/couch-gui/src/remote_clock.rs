//! Clock formatting and timezone file access stay off the render thread.
use couch_model::RemoteSettings;
use std::{
    path::Path,
    process::Command,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub struct Clock {
    rx: mpsc::Receiver<(String, RemoteSettings)>,
}
fn timezone_file(zone: &str) -> Option<String> {
    if zone.is_empty()
        || zone
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || !zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_+-".contains(&b))
    {
        return None;
    }
    [
        "/usr/share/zoneinfo",
        "/mnt/alpine/usr/share/zoneinfo",
        "/var/db/timezone/zoneinfo",
    ]
    .into_iter()
    .map(|root| format!("{root}/{zone}"))
    .find(|p| Path::new(p).is_file())
}
fn display(settings: &RemoteSettings) -> Option<String> {
    let mut command = Command::new("date");
    command.arg(if settings.clock_24h {
        "+%H:%M"
    } else {
        "+%I:%M %p"
    });
    if let Some(file) = timezone_file(&settings.timezone) {
        command.env("TZ", format!(":{file}"));
    }
    let out = command.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    Some(if settings.clock_24h {
        text
    } else {
        text.trim_start_matches('0').to_string()
    })
}
impl Clock {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut previous = None;
            loop {
                let settings = crate::connections::config()
                    .map(|c| c.remote)
                    .unwrap_or_default();
                let minute = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    / 60;
                let key = (minute, settings.clone());
                if previous.as_ref() != Some(&key) {
                    if let Some(clock) = display(&settings) {
                        if tx.send((clock, settings)).is_err() {
                            return;
                        }
                        previous = Some(key);
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        Self { rx }
    }
    pub fn poll(&self) -> Option<(String, RemoteSettings)> {
        self.rx.try_iter().last()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timezone_paths_cannot_escape_zoneinfo() {
        for zone in [
            "../etc/passwd",
            "/etc/passwd",
            "America/../../etc/passwd",
            "America//New_York",
        ] {
            assert!(timezone_file(zone).is_none());
        }
        assert!(timezone_file("America/New_York").is_some());
    }
}
