//! Device-scoped IR edits publish a fresh codeset before attaching it in one
//! revision-checked configuration commit. Existing referenced files never change.
use super::{ir, parse, store_error, Api, Reply, Store};
use couch_model::{Device, DeviceIr, DeviceKind, Id, Integration};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    #[serde(default)]
    kind: DeviceKind,
    text: String,
}
fn directory(store: &Store) -> std::path::PathBuf {
    store
        .path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("ir")
}
fn new_codeset(directory: &Path, text: &str) -> Result<String, String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    loop {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Clock is unavailable")?
            .as_nanos();
        let id = format!(
            "device-{time:x}-{:x}-{:x}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        if std::fs::symlink_metadata(directory.join(format!("{id}.codeset"))).is_ok() {
            continue;
        }
        ir::save(directory, &id, text)?;
        return Ok(id);
    }
}
fn require_revision(store: &Store, expected: Option<u64>) -> Result<(), Reply> {
    match expected {
        None => Err(Reply::error(
            428,
            "Reload the configuration before saving IR commands (If-Match is required)",
        )),
        Some(expected) if expected != store.revision() => {
            Err(store_error(&crate::store::Error::Stale {
                expected,
                actual: store.revision(),
            }))
        }
        _ => Ok(()),
    }
}
fn legacy_ir(device: &Device, config: &couch_model::Config) -> bool {
    matches!(
        config.resolve_integration(&device.integration),
        Some(Integration::Ir { .. })
    )
}
fn route(
    store: &mut Store,
    method: &str,
    room: &str,
    device: &str,
    body: &[u8],
    expected: Option<u64>,
) -> Reply {
    let room = Id::new(room);
    let device = Id::new(device);
    let Some(current) = store.config().room(&room).and_then(|r| r.device(&device)) else {
        return Reply::error(404, "Device no longer exists");
    };
    let legacy = legacy_ir(current, store.config());
    let dir = directory(store);
    if method == "GET" {
        let Some(codeset) = current.effective_ir_codeset(store.config()) else {
            return Reply::json(
                200,
                &json!({"codeset":null,"text":"","commands":[],"supplemental":false}),
            )
            .at(store.revision());
        };
        let mut value=ir::installed(&dir,codeset).unwrap_or_else(|_|json!({"text":"","commands":[],"error":"The assigned IR codeset is missing or invalid; replace it or remove IR commands"}));
        value["codeset"] = json!(codeset);
        value["supplemental"] = json!(current.ir.is_some());
        value["legacy"] = json!(legacy);
        return Reply::json(200, &value).at(store.revision());
    }
    if let Err(reply) = require_revision(store, expected) {
        return reply;
    }
    let codeset = if method == "PUT" {
        let edit: Edit = match parse(body) {
            Ok(v) => v,
            Err(r) => return r,
        };
        match new_codeset(&dir, &edit.text) {
            Ok(id) => Some(id),
            Err(e) => return Reply::error(400, e),
        }
    } else {
        None
    };
    let result = store.mutate(expected, |config| {
        let slot = config.room_mut(&room).unwrap().device_mut(&device).unwrap();
        slot.ir = codeset.as_ref().map(|id| DeviceIr {
            codeset: id.clone(),
        });
        if legacy {
            slot.integration = Integration::None;
        }
    });
    match result {
        Ok(()) => Reply::json(200, store.config()).at(store.revision()),
        Err(e) => {
            if let Some(id) = codeset {
                let _ = std::fs::remove_file(dir.join(format!("{id}.codeset")));
            }
            store_error(&e)
        }
    }
}
fn create(store: &mut Store, room: &str, body: &[u8], expected: Option<u64>) -> Reply {
    if let Err(reply) = require_revision(store, expected) {
        return reply;
    }
    let room = Id::new(room);
    if store.config().room(&room).is_none() {
        return Reply::error(404, "Room no longer exists");
    }
    let new: Create = match parse(body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let dir = directory(store);
    let codeset = match new_codeset(&dir, &new.text) {
        Ok(v) => v,
        Err(e) => return Reply::error(400, e),
    };
    let id = store
        .config()
        .fresh_device_id(&format!("{room}-{}", new.name));
    let result = store.mutate(expected, |config| {
        let mut device = Device::new(id.clone(), new.name, new.kind);
        device.ir = Some(DeviceIr {
            codeset: codeset.clone(),
        });
        config.room_mut(&room).unwrap().devices.push(device);
    });
    match result {
        Ok(()) => Reply::json(200, store.config())
            .at(store.revision())
            .created(&id),
        Err(e) => {
            let _ = std::fs::remove_file(dir.join(format!("{codeset}.codeset")));
            store_error(&e)
        }
    }
}
impl Api {
    pub(super) fn device_ir(
        &self,
        method: &str,
        room: &str,
        device: &str,
        body: &[u8],
        expected: Option<u64>,
    ) -> Reply {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        route(&mut store, method, room, device, body, expected)
    }
    pub(super) fn create_ir_device(&self, room: &str, body: &[u8], expected: Option<u64>) -> Reply {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        create(&mut store, room, body, expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (std::path::PathBuf, Store) {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "couch-device-ir-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = Store::open(dir.join("config.json")).unwrap();
        store
            .mutate(None, |c| {
                c.rooms.push(couch_model::Room {
                    id: "office".into(),
                    name: "Office".into(),
                    icon: None,
                    devices: vec![
                        Device::new("tv".into(), "TV", DeviceKind::Tv).with_integration(
                            Integration::Kodi {
                                host: "192.0.2.1".into(),
                                port: 9090,
                            },
                        ),
                    ],
                })
            })
            .unwrap();
        (dir, store)
    }
    fn dev(store: &Store) -> &Device {
        store
            .config()
            .room(&"office".into())
            .unwrap()
            .device(&"tv".into())
            .unwrap()
    }
    fn files(dir: &Path) -> usize {
        std::fs::read_dir(dir.join("ir"))
            .map(|r| r.count())
            .unwrap_or(0)
    }
    #[test]
    fn attachment_preserves_network_and_snapshots_are_isolated() {
        let (dir, mut store) = fixture();
        let original = dev(&store).integration.clone();
        ir::save(&dir.join("ir"), "shared", "power nec 4 8\n").unwrap();
        store
            .mutate(None, |c| {
                c.rooms[0].devices[0].ir = Some(DeviceIr {
                    codeset: "shared".into(),
                });
                let mut alias = c.rooms[0].devices[0].clone();
                alias.id = "second-tv".into();
                alias.name = "Second TV".into();
                c.rooms[0].devices.push(alias);
            })
            .unwrap();
        let revision = store.revision();
        assert_eq!(
            route(
                &mut store,
                "PUT",
                "office",
                "tv",
                br#"{"text":"power:on nec 4 196\npower:off nec 4 197\n"}"#,
                Some(revision)
            )
            .status,
            200
        );
        let first = dev(&store).ir.as_ref().unwrap().codeset.clone();
        assert_ne!(first, "shared");
        assert_eq!(
            store.config().rooms[0].devices[1].effective_ir_codeset(store.config()),
            Some("shared")
        );
        assert_eq!(dev(&store).integration, original);
        assert_eq!(
            ir::installed(&dir.join("ir"), "shared").unwrap()["text"],
            "power nec 0x4 0x8\n"
        );
        let revision = store.revision();
        assert_eq!(
            route(
                &mut store,
                "PUT",
                "office",
                "tv",
                br#"{"text":"mute nec 4 9"}"#,
                Some(revision)
            )
            .status,
            200
        );
        assert_ne!(dev(&store).ir.as_ref().unwrap().codeset, first);
        assert!(ir::installed(&dir.join("ir"), &first).unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("power:on"));
        let reply = route(&mut store, "GET", "office", "tv", b"", None);
        let value: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(value["commands"][0]["name"], "mute");
        assert_eq!(value["supplemental"], true);
        let revision = store.revision();
        assert_eq!(
            route(&mut store, "DELETE", "office", "tv", b"", Some(revision)).status,
            200
        );
        assert!(dev(&store).ir.is_none());
        assert_eq!(dev(&store).integration, original);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn stale_missing_revision_or_invalid_commands_leave_config_and_files_untouched() {
        let (dir, mut store) = fixture();
        let before = store.config().clone();
        let revision = store.revision();
        for (expected, body, status) in [
            (None, br#"{"text":"power nec 4 8"}"#.as_slice(), 428),
            (
                Some(revision - 1),
                br#"{"text":"power nec 4 8"}"#.as_slice(),
                409,
            ),
            (
                Some(revision),
                br#"{"text":"power unknown 4 8"}"#.as_slice(),
                400,
            ),
        ] {
            assert_eq!(
                route(&mut store, "PUT", "office", "tv", body, expected).status,
                status
            );
            assert_eq!(store.config(), &before);
            assert_eq!(files(&dir), 0);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn create_is_atomic_and_legacy_delete_does_not_restore_old_codes() {
        let (dir, mut store) = fixture();
        let revision = store.revision();
        let reply = create(
            &mut store,
            "office",
            br#"{"name":"Receiver","kind":"speaker","text":"volume:up nec 4 2"}"#,
            Some(revision),
        );
        assert_eq!(reply.status, 200);
        assert!(reply.created.is_some());
        let created = store.config().rooms[0].devices.last().unwrap();
        assert_eq!(created.integration, Integration::None);
        assert!(created.ir.is_some());
        let count = files(&dir);
        let before = store.config().clone();
        let revision = store.revision();
        assert_eq!(
            create(
                &mut store,
                "office",
                br#"{"name":" ","text":"power nec 4 8"}"#,
                Some(revision)
            )
            .status,
            422
        );
        assert_eq!(files(&dir), count);
        assert_eq!(store.config(), &before);
        store
            .mutate(None, |c| {
                c.rooms[0].devices[0].integration = Integration::Ir {
                    codeset: "old".into(),
                }
            })
            .unwrap();
        let revision = store.revision();
        assert_eq!(
            route(&mut store, "DELETE", "office", "tv", b"", Some(revision)).status,
            200
        );
        assert!(dev(&store).effective_ir_codeset(store.config()).is_none());
        assert_eq!(dev(&store).integration, Integration::None);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn persistence_failure_removes_unpublished_codeset_and_restores_config() {
        let (dir, mut store) = fixture();
        let before = store.config().clone();
        let revision = store.revision();
        std::fs::remove_file(store.path()).unwrap();
        std::fs::create_dir(store.path()).unwrap();
        assert_eq!(
            route(
                &mut store,
                "PUT",
                "office",
                "tv",
                br#"{"text":"power nec 4 8"}"#,
                Some(revision)
            )
            .status,
            500
        );
        assert_eq!(store.config(), &before);
        assert_eq!(files(&dir), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
