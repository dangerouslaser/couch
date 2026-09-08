//! The config file, and the only code allowed to write it.
//!
//! Every mutation goes through [`Store::mutate`], which validates the result
//! *before* it replaces the in-memory copy and only then writes the file. A
//! rejected edit therefore changes nothing at all, rather than leaving the
//! daemon serving a document that the next reader would refuse to load.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use couch_model::{Config, ValidationError};

pub const DEFAULT_PATH: &str = "/opt/couch/config.json";

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Parse(serde_json::Error),
    Invalid(ValidationError),
    /// The client sent an `If-Match` that no longer matches.
    Stale { expected: u64, actual: u64 },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Parse(e) => write!(f, "not valid JSON: {e}"),
            Error::Invalid(e) => write!(f, "{e}"),
            Error::Stale { expected, actual } => write!(
                f,
                "config changed underneath you (you had revision {expected}, it is now {actual})"
            ),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

pub struct Store {
    path: PathBuf,
    config: Config,
}

impl Store {
    /// Load, or seed.
    ///
    /// A missing file is the first boot and gets the seed house written out, so
    /// the web UI opens on something rather than an empty list a user has to
    /// guess their way out of. A *corrupt* file is a different matter and is an
    /// error: overwriting somebody's house because one brace is missing is not
    /// a recovery, it is data loss with extra steps.
    pub fn open(path: impl Into<PathBuf>) -> Result<Store, Error> {
        let path = path.into();
        match fs::read(&path) {
            Ok(bytes) => {
                let config: Config = serde_json::from_slice(&bytes).map_err(Error::Parse)?;
                config.validate().map_err(Error::Invalid)?;
                let legacy = serde_json::from_slice::<serde_json::Value>(&bytes).ok().is_some_and(|v|v.get("connections").is_none());
                let mut store = Store { path, config };
                if legacy {
                    let parent=store.path.parent().unwrap_or_else(||Path::new("."));
                    for (file, name, provider) in [("ha-connection.json","Home Assistant",couch_model::Provider::HomeAssistant),("hue-connection.json","Philips Hue",couch_model::Provider::Hue)] {
                        if parent.join(file).is_file() {
                            store.config.connections.push(couch_model::Connection{id:couch_model::Id::from_name(name),name:name.into(),provider});
                        }
                    }
                    if !store.config.connections.is_empty() {
                        store.config.revision=store.config.revision.wrapping_add(1);
                        store.config.validate().map_err(Error::Invalid)?;
                        store.write()?;
                    }
                }
                Ok(store)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let store = Store { path, config: Config::seed() };
                store.write()?;
                Ok(store)
            }
            Err(e) => Err(Error::Io(e)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn revision(&self) -> u64 {
        self.config.revision
    }

    /// Apply `f` to a copy, and keep it only if the result validates.
    ///
    /// `if_match` is the revision the client believed it was editing. Two
    /// phones on the same page is the collision that actually happens here, and
    /// without this the second save silently wins.
    pub fn mutate<T>(
        &mut self,
        if_match: Option<u64>,
        f: impl FnOnce(&mut Config) -> T,
    ) -> Result<T, Error> {
        if let Some(expected) = if_match {
            if expected != self.config.revision {
                return Err(Error::Stale { expected, actual: self.config.revision });
            }
        }
        let mut next = self.config.clone();
        let out = f(&mut next);
        next.revision = self.config.revision.wrapping_add(1);
        next.validate().map_err(Error::Invalid)?;

        let previous = std::mem::replace(&mut self.config, next);
        if let Err(e) = self.write() {
            // The file is the source of truth; if it did not take, neither did
            // the edit, or a restart would quietly undo what the UI just
            // confirmed.
            self.config = previous;
            return Err(e);
        }
        Ok(out)
    }

    /// Write, atomically.
    ///
    /// Temp file in the same directory, fsync, rename. Same directory matters:
    /// rename is only atomic within a filesystem, and `/tmp` on this device is
    /// tmpfs while `/opt/couch` is the rootfs partition. The fsync matters
    /// because the realistic way this device stops is a battery pull or a
    /// watchdog reset, and rename ordering without it can leave a
    /// present-but-empty file - which is exactly the state `open` above refuses
    /// to recover from.
    fn write(&self) -> Result<(), Error> {
        if let Some(dir) = self.path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        let tmp = self.path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(&self.config).map_err(Error::Parse)?;
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(&json)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;
        // Durably record the rename too, not just the bytes it points at.
        if let Some(dir) = self.path.parent() {
            if !dir.as_os_str().is_empty() {
                if let Ok(handle) = fs::File::open(dir) {
                    let _ = handle.sync_all();
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use couch_model::Id;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("couch-confd-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        dir.join("config.json")
    }

    #[test]
    fn a_missing_file_is_seeded() {
        let path = scratch("seed");
        let store = Store::open(&path).unwrap();
        assert!(!store.config().rooms.is_empty());
        assert!(path.exists());
        // And it reloads to the same thing.
        let again = Store::open(&path).unwrap();
        assert_eq!(again.config(), store.config());
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_a_reset() {
        let path = scratch("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ not json").unwrap();
        assert!(matches!(Store::open(&path), Err(Error::Parse(_))));
    }

    #[test]
    fn a_rejected_edit_changes_nothing() {
        let path = scratch("reject");
        let mut store = Store::open(&path).unwrap();
        let before = store.config().clone();
        let err = store
            .mutate(None, |cfg| cfg.rooms.push(cfg.rooms[0].clone()))
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
        assert_eq!(store.config().revision, before.revision);
        assert_eq!(store.config().rooms.len(), before.rooms.len());
        assert_eq!(Store::open(&path).unwrap().config(), &before);
    }

    #[test]
    fn revisions_advance_and_gate_writes() {
        let path = scratch("revision");
        let mut store = Store::open(&path).unwrap();
        let r0 = store.revision();
        store.mutate(Some(r0), |cfg| cfg.remove_room(&Id::new("loft"))).unwrap();
        assert_eq!(store.revision(), r0 + 1);
        let err = store.mutate(Some(r0), |_| {}).unwrap_err();
        assert!(matches!(err, Error::Stale { .. }));
    }

    #[test]
    fn writes_leave_no_temp_file_behind() {
        let path = scratch("atomic");
        let mut store = Store::open(&path).unwrap();
        store.mutate(None, |cfg| cfg.remove_room(&Id::new("porch"))).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
    }
}

#[cfg(test)]
mod connection_migration_tests {
    use super::*;
    #[test]
    fn paired_bridge_is_adopted_once_without_touching_credentials() {
        let dir=std::env::temp_dir().join(format!("couch-connection-migration-{}",std::process::id()));fs::create_dir_all(&dir).unwrap();
        let path=dir.join("config.json");let credential=dir.join("hue-connection.json");
        fs::write(&path,br#"{"schema_version":1,"revision":3,"rooms":[]}"#).unwrap();fs::write(&credential,b"private fixture").unwrap();
        let mut store=Store::open(&path).unwrap();assert_eq!(store.revision(),4);assert_eq!(store.config().connections.len(),1);
        store.mutate(Some(4),|c|c.connections.clear()).unwrap();drop(store);
        assert!(Store::open(&path).unwrap().config().connections.is_empty());assert_eq!(fs::read(&credential).unwrap(),b"private fixture");fs::remove_dir_all(dir).unwrap();
    }
}
