//! Private per-connection web credentials. Never included in house exports.
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::Path};
#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub host: String,
    pub web_port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub http_control: bool,
}
impl Settings {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }
    pub fn client(&self) -> crate::Kodi {
        crate::Kodi::http(&self.host, self.web_port).with_auth(&self.username, &self.password)
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let temporary = path.with_extension(format!("{}.new", std::process::id()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec(self)?)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            fs::File::open(path.parent().unwrap())?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}
