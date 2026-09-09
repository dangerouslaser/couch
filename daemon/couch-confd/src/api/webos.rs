//! LG credentials stay private; browser responses contain status only.
use super::Reply;
use couch_webos::{settings::Settings, Client};
use serde::Deserialize;
use serde_json::json;
use std::{net::IpAddr,path::PathBuf};


#[derive(Deserialize)]
struct Setup {
    address: String,
    #[serde(default)]
    legacy: bool,
}
fn url(input: &Setup) -> Option<String> {
    let ip: IpAddr = input.address.trim().parse().ok()?;
    if ip.is_unspecified() || ip.is_multicast() || ip.is_loopback() {
        return None;
    }
    let host = match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    Some(format!(
        "{}://{host}:{}/",
        if input.legacy { "ws" } else { "wss" },
        if input.legacy { 3000 } else { 3001 }
    ))
}
pub(super) fn route(method: &str, path: &[&str], body: &[u8]) -> Reply {
    let file = PathBuf::from(
        std::env::var("COUCH_WEBOS_CONNECTION")
            .unwrap_or_else(|_| "/opt/couch/webos-connection.json".into()),
    );
    route_at(method,path,body,file)
}
pub(super) fn route_at(method: &str,path:&[&str],body:&[u8],file:PathBuf)->Reply {
    if let Some(parent)=file.parent(){if std::fs::create_dir_all(parent).is_err(){return Reply::error(500,"Cannot create private connection directory");}}
    let connection_lock=super::connections::lock_for(&file);
    let Ok(_guard) = connection_lock.try_lock() else {
        return Reply::error(
            409,
            "TV connection is busy. Finish pairing or try again shortly.",
        );
    };

    if method == "GET" && path == ["connection"] {
        return Reply::json(
            200,
            &match Settings::load(&file) {
                Ok(s) => json!({"url":s.url,"paired":true}),
                Err(_) => json!({"url":"","paired":false}),
            },
        );
    }
    if method == "PUT" && path == ["connection"] {
        let Some(url) = serde_json::from_slice::<Setup>(body)
            .ok()
            .and_then(|s| url(&s))
        else {
            return Reply::error(400, "Enter the TV’s IPv4 or IPv6 address");
        };
        // A normal test never triggers pairing or changes the pinned certificate.
        let (mut client, settings) = match Client::pair(&url) {
            Ok(pair) => pair,
            Err(e) => return Reply::error(502, e.to_string()),
        };
        if let Err(e) = client.power_state() {
            return Reply::error(502, e.to_string());
        }
        if settings.save(&file).is_err() {
            return Reply::error(
                500,
                "TV paired, but credentials could not be saved. Try pairing again.",
            );
        }
        return Reply::json(200, &json!({"url":settings.url,"paired":true}));
    }
    if !matches!(
        (method, path),
        ("GET", ["status" | "inputs" | "apps"]) | ("POST", ["command"])
    ) {
        return Reply::error(404, "Unknown TV operation");
    }
    let command = if method == "POST" {
        match serde_json::from_slice::<Action>(body) {
            Ok(action) => Some(action),
            Err(_) => return Reply::error(400, "Invalid TV command"),
        }
    } else {
        None
    };
    let settings = match Settings::load(&file) {
        Ok(s) => s,
        Err(_) => return Reply::error(400, "Pair the LG webOS TV in Connections first"),
    };
    let mut client = match Client::connect(&settings) {
        Ok(c) => c,
        Err(e) => {
            return Reply::error(
                502,
                format!("TV unavailable: {e}. Check that it is on and connected."),
            )
        }
    };
    let result = match command {
        Some(Action::VolumeUp) => client.volume_up().map(|_| json!({"accepted":true})),
        Some(Action::VolumeDown) => client.volume_down().map(|_| json!({"accepted":true})),
        Some(Action::Mute { on }) => client.mute(on).map(|_| json!({"accepted":true})),
        Some(Action::Input { id }) => client.select_input(&id).map(|_| json!({"accepted":true})),
        Some(Action::Launch { id }) => client.launch_app(&id).map(|_| json!({"accepted":true})),
        None => match path {
            ["inputs"] => client.inputs(),
            ["apps"] => client.apps(),
            _ => (|| {
                Ok(
                    json!({"power":client.power_state()?,"volume":client.volume()?,"app":client.foreground_app()?}),
                )
            })(),
        },
    };
    match result {
        Ok(v) => Reply::json(200, &v),
        Err(e) => Reply::error(502, e.to_string()),
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
enum Action {
    VolumeUp,
    VolumeDown,
    Mute { on: bool },
    Input { id: String },
    Launch { id: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tv_address_uses_tls_unless_legacy_is_explicit() {
        assert_eq!(
            url(&Setup {
                address: "192.168.1.176".into(),
                legacy: false
            })
            .unwrap(),
            "wss://192.168.1.176:3001/"
        );
        assert_eq!(
            url(&Setup {
                address: "192.168.1.176".into(),
                legacy: true
            })
            .unwrap(),
            "ws://192.168.1.176:3000/"
        );
        for address in [
            "localhost",
            "127.0.0.1",
            "0.0.0.0",
            "ws://192.168.1.176/",
            "224.0.0.1",
        ] {
            assert!(url(&Setup {
                address: address.into(),
                legacy: false
            })
            .is_none());
        }
    }
    #[test]
    fn commands_are_explicit_and_reject_unknown_actions() {
        assert!(serde_json::from_str::<Action>(r#"{"action":"mute","on":false}"#).is_ok());
        assert!(serde_json::from_str::<Action>(r#"{"action":"mute"}"#).is_err());
        assert!(
            serde_json::from_str::<Action>(r#"{"action":"request","uri":"anything"}"#).is_err()
        );
    }
}
