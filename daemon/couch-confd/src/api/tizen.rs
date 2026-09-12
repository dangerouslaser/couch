//! Samsung Tizen pairing and test controls. The token, certificate and MAC
//! stay in the private connection file; browser replies carry status only.
//! Nothing here has run against a physical TV yet: see docs/samsung-tizen.md.
use super::Reply;
use couch_control::{StreamingConnection, StreamingTv};
use couch_tizen::{rest, Client, Settings};
use serde::Deserialize;
use serde_json::json;
use std::{net::IpAddr, path::PathBuf, time::Duration};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Setup {
    address: String,
    #[serde(default)]
    legacy: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    command: String,
}
fn address(input: &str) -> Option<IpAddr> {
    let address: IpAddr = input.trim().parse().ok()?;
    (!address.is_unspecified() && !address.is_multicast() && !address.is_loopback())
        .then_some(address)
}
fn summary(settings: &Settings) -> Reply {
    Reply::json(
        200,
        &json!({"paired":true,"address":settings.address().map(|a|a.to_string()).unwrap_or_default(),
            "secure":settings.secure(),"model":settings.model,"name":settings.name,
            "wake_supported":settings.mac.is_some(),"frame_tv":settings.frame_tv,
            "token_issued":!settings.token.is_empty(),"experimental":true}),
    )
}
fn discover() -> Reply {
    let found = match couch_tizen::discovery::search(Duration::from_secs(3)) {
        Ok(found) => found,
        Err(_) => {
            return Reply::error(
                503,
                "Network discovery is unavailable; enter the TV address manually",
            )
        }
    };
    let tvs: Vec<_> = found
        .into_iter()
        .take(16)
        .filter_map(|address| {
            // Only a TV that answers its own information endpoint is listed.
            let info = rest::device_info(address, Duration::from_secs(2)).ok()?;
            Some(json!({"name":info.name,"address":address.to_string(),"model":info.model}))
        })
        .collect();
    Reply::json(200, &tvs)
}
fn load(file: &std::path::Path) -> Option<Settings> {
    match StreamingConnection::load(file) {
        Ok(StreamingConnection::Tizen { settings }) => Some(settings),
        _ => None,
    }
}
pub(super) fn route(method: &str, path: &[&str], body: &[u8], file: PathBuf) -> Reply {
    if method == "GET" && path == ["discover"] {
        return discover();
    }
    if method == "GET" && path == ["inputs"] {
        // Fixed source keys; the same shape as the LG input list so the
        // activity pickers need no special case.
        return Reply::json(
            200,
            &json!({"devices":couch_tizen::INPUTS.iter().map(|(id,label)|json!({"id":id,"label":label})).collect::<Vec<_>>()}),
        );
    }
    if let Some(parent) = file.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return Reply::error(500, "Cannot create private connection directory");
        }
    }
    let connection_lock = super::connections::lock_for(&file);
    let Ok(_guard) = connection_lock.try_lock() else {
        return Reply::error(
            409,
            "TV connection is busy. Finish pairing or try again shortly.",
        );
    };
    if method == "GET" && path == ["connection"] {
        return match load(&file) {
            Some(settings) => summary(&settings),
            None => Reply::json(200, &json!({"paired":false,"experimental":true})),
        };
    }
    if method == "PUT" && path == ["connection"] {
        let Some(address) = serde_json::from_slice::<Setup>(body)
            .ok()
            .and_then(|s| address(&s.address).map(|a| (a, s.legacy)))
        else {
            return Reply::error(400, "Enter the TV’s IPv4 or IPv6 address");
        };
        let (address, legacy) = address;
        // The information endpoint needs no pairing and tells us whether the
        // TV issues tokens, its MAC for waking and whether it is The Frame.
        let info = rest::device_info(address, Duration::from_secs(4)).ok();
        let secure = !legacy && info.as_ref().is_none_or(|i| i.token_auth);
        let (_, mut settings) = match Client::pair(&couch_tizen::base_url(address, secure)) {
            Ok(pair) => pair,
            Err(e) => return Reply::error(502, e.to_string()),
        };
        if let Some(info) = info {
            settings.mac = info.mac;
            settings.frame_tv = info.frame_tv;
            settings.model = info.model;
            settings.name = info.name;
        }
        let connection = StreamingConnection::Tizen { settings };
        if let Some(parent) = file.parent() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
        if connection.save(&file).is_err() {
            return Reply::error(
                500,
                "TV paired, but credentials could not be saved. Try pairing again.",
            );
        }
        return match connection {
            StreamingConnection::Tizen { settings } => summary(&settings),
            _ => unreachable!(),
        };
    }
    if !matches!(
        (method, path),
        ("GET", ["status" | "apps"]) | ("POST", ["command"])
    ) {
        return Reply::error(404, "Unknown TV operation");
    }
    let command = if method == "POST" {
        let Ok(command) = serde_json::from_slice::<Command>(body) else {
            return Reply::error(400, "Invalid TV command");
        };
        if !couch_model::commands::Function::parse(&command.command)
            .is_some_and(|f| f.supports(&couch_model::Integration::Tizen))
        {
            return Reply::error(400, "Unsupported TV command");
        }
        Some(command.command)
    } else {
        None
    };
    let connection = match StreamingConnection::load(&file) {
        Ok(c) if c.kind() == "tizen" => c,
        Ok(_) => return Reply::error(400, "Saved credentials do not match this connection type"),
        Err(_) => return Reply::error(400, "Pair this Samsung TV in Connections first"),
    };
    // Waking must not require the socket the sleeping TV cannot answer.
    if command.as_deref() == Some("power-on") {
        let StreamingConnection::Tizen { settings } = &connection else {
            unreachable!()
        };
        return match couch_tizen::wake_paired(settings) {
            Ok(()) => Reply::json(
                200,
                &json!({"accepted":true,"note":"Wake packet sent; the TV's power state is not confirmed"}),
            ),
            Err(couch_tizen::Error::Unsupported) => Reply::error(
                400,
                "The TV did not report a MAC address while pairing; pair again with it on",
            ),
            Err(e) => Reply::error(502, e.to_string()),
        };
    }
    let client = match StreamingTv::connect(&connection) {
        Ok(c) => c,
        Err(e) => {
            return Reply::error(
                502,
                format!("TV unavailable: {e}. Check that it is on and allowed Couch."),
            )
        }
    };
    let result = match command {
        Some(command) => client.command(&command).map(|_| json!({"accepted":true})),
        None if path == ["apps"] => client.apps(),
        None => client.status(),
    };
    match result {
        Ok(value) => Reply::json(200, &value),
        Err(e) => Reply::error(502, e.to_string()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "couch-tizen-api-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn only_literal_lan_addresses_are_accepted() {
        for bad in [
            "localhost",
            "127.0.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "::1",
            "ws://192.168.1.2/",
        ] {
            assert!(address(bad).is_none());
        }
        assert!(address(" 192.168.1.2 ").is_some());
    }
    #[test]
    fn unpaired_routes_and_inputs_need_no_tv() {
        let root = root();
        let file = root.join("tizen-connection.json");
        let reply = route("GET", &["connection"], b"", file.clone());
        assert_eq!(reply.status, 200);
        let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(body["paired"], false);
        assert_eq!(route("GET", &["status"], b"", file.clone()).status, 400);
        assert_eq!(route("GET", &["apps"], b"", file.clone()).status, 400);
        assert_eq!(route("POST", &["apps"], b"", file.clone()).status, 404);
        assert_eq!(
            route(
                "PUT",
                &["connection"],
                br#"{"address":"localhost"}"#,
                file.clone()
            )
            .status,
            400
        );
        let inputs = route("GET", &["inputs"], b"", file.clone());
        let body: serde_json::Value = serde_json::from_slice(&inputs.body).unwrap();
        assert_eq!(body["devices"][2]["id"], "hdmi1");
        assert_eq!(
            route("POST", &["command"], br#"{"command":"next"}"#, file.clone()).status,
            400
        );
        assert_eq!(
            route("POST", &["command"], br#"{"command":"ok"}"#, file).status,
            400,
            "unpaired TV rejects before any socket is opened"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn status_never_exposes_token_and_wake_needs_a_mac() {
        let root = root();
        let file = root.join("tizen-connection.json");
        let connection = StreamingConnection::Tizen {
            settings: Settings {
                url: "wss://192.0.2.9:8002/".into(),
                token: "secrettoken".into(),
                certificate: vec![1],
                mac: None,
                frame_tv: false,
                model: "QE55".into(),
                name: "[TV] Test".into(),
            },
        };
        connection.save(&file).unwrap();
        let reply = route("GET", &["connection"], b"", file.clone());
        assert_eq!(reply.status, 200);
        let text = String::from_utf8(reply.body).unwrap();
        assert!(!text.contains("secrettoken"));
        assert!(text.contains("192.0.2.9"));
        assert!(text.contains("\"wake_supported\":false"));
        let reply = route(
            "POST",
            &["command"],
            br#"{"command":"power-on"}"#,
            file.clone(),
        );
        assert_eq!(reply.status, 400);
        assert_eq!(
            route("POST", &["command"], br#"{"command":"garbage"}"#, file).status,
            400
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
