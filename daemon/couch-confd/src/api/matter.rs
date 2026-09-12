//! Matter fabric routes, scoped to one connection's private directory. The
//! browser sees node inventories and light states; keys never leave the remote.
use super::{parse, Reply};
use couch_matter::{Command, Controller, Error};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Commission {
    code: String,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rename {
    name: String,
}
#[derive(Deserialize)]
struct Action {
    action: String,
    brightness: Option<u8>,
}

/// One open controller per fabric directory: opening binds sockets and loads
/// keys, and every request would otherwise pay that again.
fn controller(dir: &Path) -> Result<Arc<Controller>, Error> {
    static OPEN: OnceLock<Mutex<HashMap<PathBuf, Arc<Controller>>>> = OnceLock::new();
    let mut open = OPEN.get_or_init(Default::default).lock().unwrap();
    if let Some(c) = open.get(dir) {
        return Ok(c.clone());
    }
    let c = Arc::new(Controller::open(dir)?);
    open.insert(dir.to_path_buf(), c.clone());
    Ok(c)
}

fn reply<T: serde::Serialize>(result: Result<T, Error>) -> Reply {
    match result {
        Ok(v) => Reply::json(200, &v),
        Err(Error::Invalid(m)) => Reply::error(400, m),
        Err(Error::NotFound(m)) => Reply::error(404, m),
        Err(e @ Error::Timeout(_)) => Reply::error(504, e.to_string()),
        Err(e @ Error::Device(_)) => Reply::error(502, e.to_string()),
        Err(e @ Error::Storage(_)) => Reply::error(500, e.to_string()),
    }
}

pub(super) fn route_at(method: &str, path: &[&str], body: &[u8], dir: PathBuf) -> Reply {
    // A fabric is created on the first request that needs it; a saved
    // connection with no paired devices should not bind sockets on every GET.
    if method == "GET" && path == ["connection"] && !Controller::exists(&dir) {
        return Reply::json(200, &json!({"fabric_set":false,"devices":0}));
    }
    // Commissioning takes up to a minute and rewrites the inventory; reads and
    // light commands stay concurrent since the controller is thread-safe.
    let exclusive = matches!(
        (method, path),
        ("POST", ["commission"]) | ("DELETE", ["devices", _]) | ("POST", ["devices", _, "refresh"])
    );
    let lock = super::connections::lock_for(&dir);
    let guard = if exclusive {
        match lock.try_lock() {
            Ok(g) => Some(g),
            Err(_) => {
                return Reply::error(
                    503,
                    "The Matter connection is busy pairing; try again shortly",
                )
            }
        }
    } else {
        None
    };
    let controller = match controller(&dir) {
        Ok(c) => c,
        Err(e) => return reply::<()>(Err(e)),
    };
    let node =
        |s: &str| -> Option<u64> { s.parse::<u64>().ok().filter(|n| *n != 0 && s.len() <= 20) };
    let result = match (method, path) {
        ("GET", ["connection"]) => Reply::json(
            200,
            &json!({"fabric_set":true,"fabric_id":format!("{:016X}",controller.fabric_id()),"devices":controller.nodes().len()}),
        ),
        ("POST", ["discover"]) => reply(controller.discover()),
        ("POST", ["commission"]) => {
            let input: Commission = match parse(body) {
                Ok(v) => v,
                Err(r) => return r,
            };
            reply(controller.commission(&input.code, &input.name))
        }
        ("GET", ["devices"]) => Reply::json(200, &controller.nodes()),
        ("PUT", ["devices", id]) => match node(id) {
            Some(id) => {
                let input: Rename = match parse(body) {
                    Ok(v) => v,
                    Err(r) => return r,
                };
                reply(controller.rename(id, &input.name))
            }
            None => Reply::error(404, "Unknown Matter device"),
        },
        ("POST", ["devices", id, "refresh"]) => match node(id) {
            Some(id) => reply(controller.refresh(id)),
            None => Reply::error(404, "Unknown Matter device"),
        },
        ("DELETE", ["devices", id]) => match node(id) {
            Some(id) => reply(
                controller
                    .remove(id)
                    .map(|released| json!({"removed":true,"fabric_released":released})),
            ),
            None => Reply::error(404, "Unknown Matter device"),
        },
        ("GET", ["lights"]) => Reply::json(200, &controller.lights()),
        ("GET", ["lights", node, endpoint]) => {
            reply(controller.light(&format!("{node}/{endpoint}")))
        }
        ("POST", ["lights", node, endpoint, "command"]) => {
            let Ok(action) = serde_json::from_slice::<Action>(body) else {
                return Reply::error(400, "Invalid light command");
            };
            let command = match (action.action.as_str(), action.brightness) {
                ("on", None) => Command::On,
                ("off", None) => Command::Off,
                ("toggle", None) => Command::Toggle,
                ("brightness", Some(p)) if p <= 100 => Command::Brightness(p),
                _ => {
                    return Reply::error(400, "Choose on, off, toggle or brightness from 0 to 100")
                }
            };
            reply(controller.command(&format!("{node}/{endpoint}"), command))
        }
        _ => Reply::error(404, "Unknown Matter operation"),
    };
    drop(guard);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unpaired_connection_answers_without_creating_a_fabric_and_inputs_are_checked_first() {
        let dir = std::env::temp_dir().join(format!("matter-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let status = route_at("GET", &["connection"], &[], dir.clone());
        assert_eq!(status.status, 200);
        assert!(!dir.exists());
        assert_eq!(route_at("GET", &["nothing"], &[], dir.clone()).status, 404);
        // A typo is rejected before any pairing attempt, and the fabric is created.
        let bad = route_at(
            "POST",
            &["commission"],
            br#"{"code":"3497-011-2333","name":"Lamp"}"#,
            dir.clone(),
        );
        assert_eq!(bad.status, 400);
        assert!(Controller::exists(&dir));
        assert_eq!(
            route_at(
                "POST",
                &["commission"],
                br#"{"code":"3497-011-2332"}"#,
                dir.clone()
            )
            .status,
            400
        );
        assert_eq!(
            route_at(
                "POST",
                &["lights", "1", "1", "command"],
                br#"{"action":"volume"}"#,
                dir.clone()
            )
            .status,
            400
        );
        assert_eq!(
            route_at(
                "POST",
                &["lights", "1", "1", "command"],
                br#"{"action":"on"}"#,
                dir.clone()
            )
            .status,
            404
        );
        assert_eq!(
            route_at("DELETE", &["devices", "0"], &[], dir.clone()).status,
            404
        );
        assert_eq!(
            route_at("DELETE", &["devices", "9"], &[], dir.clone()).status,
            404
        );
        let status = route_at("GET", &["connection"], &[], dir.clone());
        let value: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
        assert_eq!(value["fabric_set"], true);
        assert_eq!(value["devices"], 0);
        assert_eq!(value["fabric_id"].as_str().unwrap().len(), 16);
        let lights = route_at("GET", &["lights"], &[], dir.clone());
        assert_eq!(String::from_utf8(lights.body).unwrap(), "[]");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
