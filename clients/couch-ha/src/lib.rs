//! Blocking Home Assistant REST client. Call from a worker, never the render loop.
//! Commands use services, not POST /states (which only changes HA's state cache).
pub mod settings;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fmt, time::Duration};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Configuration,
    Authentication,
    Missing,
    Unavailable,
    UnsupportedBrightness,
    InvalidBrightness,
    Transport,
    Response,
    Status(u16),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "Check the Home Assistant URL, token and light entity ID",
            Self::Authentication => "Home Assistant rejected the access token",
            Self::Missing => "The light no longer exists in Home Assistant",
            Self::Unavailable => "The light is unavailable or its state is unknown",
            Self::UnsupportedBrightness => "This light does not support brightness",
            Self::InvalidBrightness => "Brightness must be between 0 and 100 percent",
            Self::Transport => "Could not reach Home Assistant (connection, TLS or timeout)",
            Self::Response => "Home Assistant returned an invalid or oversized response",
            Self::Status(_) => "Home Assistant rejected the request",
        })
    }
}
impl std::error::Error for Error {}
fn transport(e: ureq::Error) -> Error {
    match e {
        ureq::Error::StatusCode(401 | 403) => Error::Authentication,
        ureq::Error::StatusCode(404) => Error::Missing,
        ureq::Error::StatusCode(code) => Error::Status(code),
        _ => Error::Transport,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Light {
    pub entity_id: String,
    pub name: String,
    /// None means unknown/unavailable, never an inferred off state.
    pub on: Option<bool>,
    pub brightness_percent: Option<u8>,
    pub dimmable: bool,
}
impl Light {
    pub fn from_state(v: &Value) -> Option<Self> {
        let id = v.get("entity_id")?.as_str()?;
        if !valid_light(id) {
            return None;
        }
        let on = match v.get("state")?.as_str()? {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        let attrs = &v["attributes"];
        let dimmable = if let Some(modes) = attrs["supported_color_modes"].as_array() {
            modes.iter().filter_map(Value::as_str).any(|s| {
                matches!(
                    s,
                    "brightness" | "color_temp" | "hs" | "xy" | "rgb" | "rgbw" | "rgbww" | "white"
                )
            })
        } else {
            attrs["supported_features"].as_u64().unwrap_or(0) & 1 != 0
        };
        let brightness_percent = if on == Some(false) {
            Some(0)
        } else if on == Some(true) && dimmable {
            attrs["brightness"]
                .as_u64()
                .filter(|b| *b <= 255)
                .map(|b| ((b * 100 + 127) / 255) as u8)
        } else {
            None
        };
        Some(Self {
            entity_id: id.into(),
            name: attrs["friendly_name"].as_str().unwrap_or(id).into(),
            on,
            brightness_percent,
            dimmable,
        })
    }
}
fn valid_light(id: &str) -> bool {
    id.strip_prefix("light.").is_some_and(|s| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    })
}
#[derive(Debug, Clone, Copy)]
pub enum Command {
    On,
    Off,
    Brightness(u8),
}

pub struct HomeAssistant {
    base: String,
    token: String,
    agent: ureq::Agent,
}
// Deliberately do not expose the credential through Debug or error messages.
impl fmt::Debug for HomeAssistant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HomeAssistant").finish_non_exhaustive()
    }
}
impl HomeAssistant {
    pub fn new(base: &str, token: &str) -> Result<Self> {
        Self::with_timeout(base, token, Duration::from_secs(5))
    }
    pub fn with_timeout(base: &str, token: &str, timeout: Duration) -> Result<Self> {
        let parsed = url::Url::parse(base).map_err(|_| Error::Configuration)?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || token.trim().is_empty()
            || token.bytes().any(|b| b.is_ascii_control())
            || timeout.is_zero()
        {
            return Err(Error::Configuration);
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .proxy(None)
            .build()
            .new_agent();
        Ok(Self {
            base: parsed.as_str().trim_end_matches('/').into(),
            token: token.into(),
            agent,
        })
    }
    fn get(&self, path: &str) -> Result<Value> {
        let mut r = self
            .agent
            .get(format!("{}{path}", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .call()
            .map_err(transport)?;
        if !r.status().is_success() {
            return Err(Error::Status(r.status().as_u16()));
        }
        r.body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_json()
            .map_err(|_| Error::Response)
    }
    pub fn lights(&self) -> Result<Vec<Light>> {
        let values = self.get("/api/states")?;
        let mut lights: Vec<_> = values
            .as_array()
            .ok_or(Error::Response)?
            .iter()
            .filter_map(Light::from_state)
            .collect();
        lights.sort_by(|a, b| a.name.cmp(&b.name).then(a.entity_id.cmp(&b.entity_id)));
        Ok(lights)
    }
    pub fn light(&self, id: &str) -> Result<Light> {
        if !valid_light(id) {
            return Err(Error::Configuration);
        }
        let light =
            Light::from_state(&self.get(&format!("/api/states/{id}"))?).ok_or(Error::Response)?;
        if light.entity_id != id {
            return Err(Error::Response);
        }
        Ok(light)
    }
    /// A successful return acknowledges service execution. Read state again to
    /// display confirmed values; HA integrations may update asynchronously.
    pub fn command(&self, id: &str, command: Command) -> Result<()> {
        if let Command::Brightness(p) = command {
            if p > 100 {
                return Err(Error::InvalidBrightness);
            }
        }
        let current = self.light(id)?;
        if current.on.is_none() {
            return Err(Error::Unavailable);
        }
        let (service, data) = match command {
            Command::Off | Command::Brightness(0) => ("turn_off", json!({"entity_id":id})),
            Command::On => ("turn_on", json!({"entity_id":id})),
            Command::Brightness(p) => {
                if !current.dimmable {
                    return Err(Error::UnsupportedBrightness);
                }
                ("turn_on", json!({"entity_id":id,"brightness_pct":p}))
            }
        };
        let mut reply = self
            .agent
            .post(format!("{}/api/services/light/{service}", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .send_json(&data)
            .map_err(transport)?;
        if !reply.status().is_success() {
            return Err(Error::Status(reply.status().as_u16()));
        }
        let value: Value = reply
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_json()
            .map_err(|_| Error::Response)?;
        if !value.is_array() {
            return Err(Error::Response);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(state: &str, modes: Value) -> Value {
        json!({"entity_id":"light.test","state":state,"attributes":{"friendly_name":"Test light","brightness":128,"supported_color_modes":modes}})
    }
    fn server(
        replies: Vec<(u16, Value)>,
    ) -> (
        String,
        std::thread::JoinHandle<Vec<(String, String, Value)>>,
    ) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = format!("http://{}", server.server_addr());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let mut request = server
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .expect("request deadline");
                assert!(request
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Authorization")
                        && h.value.as_str() == "Bearer test-secret"));
                let mut bytes = String::new();
                request.as_reader().read_to_string(&mut bytes).unwrap();
                requests.push((
                    request.method().as_str().into(),
                    request.url().into(),
                    serde_json::from_str(&bytes).unwrap_or(Value::Null),
                ));
                request
                    .respond(
                        tiny_http::Response::from_string(body.to_string()).with_status_code(status),
                    )
                    .unwrap();
            }
            requests
        });
        (address, handle)
    }
    #[test]
    fn discovery_filters_other_domains_and_preserves_unavailable() {
        let (url, s) = server(vec![(
            200,
            json!([state("on",json!(["brightness"])),{"entity_id":"sensor.temp","state":"42"},{"entity_id":"light.offline","state":"unavailable","attributes":{}}]),
        )]);
        let lights = HomeAssistant::new(&url, "test-secret")
            .unwrap()
            .lights()
            .unwrap();
        assert_eq!(lights.len(), 2);
        assert!(lights
            .iter()
            .any(|l| l.on == Some(true) && l.brightness_percent == Some(50)));
        assert!(lights.iter().any(|l| l.entity_id == "light.offline"
            && l.on.is_none()
            && l.brightness_percent.is_none()));
        assert_eq!(s.join().unwrap()[0].1, "/api/states");
    }
    #[test]
    fn brightness_uses_service_and_never_writes_state_cache() {
        let (url, s) = server(vec![(200, state("on", json!(["rgb"]))), (200, json!([]))]);
        HomeAssistant::new(&url, "test-secret")
            .unwrap()
            .command("light.test", Command::Brightness(37))
            .unwrap();
        let requests = s.join().unwrap();
        assert_eq!(
            requests[1],
            (
                "POST".into(),
                "/api/services/light/turn_on".into(),
                json!({"entity_id":"light.test","brightness_pct":37})
            )
        );
    }
    #[test]
    fn zero_brightness_is_explicit_off() {
        let (url, s) = server(vec![
            (200, state("on", json!(["brightness"]))),
            (200, json!([])),
        ]);
        HomeAssistant::new(&url, "test-secret")
            .unwrap()
            .command("light.test", Command::Brightness(0))
            .unwrap();
        assert_eq!(s.join().unwrap()[1].1, "/api/services/light/turn_off");
    }
    #[test]
    fn unavailable_and_non_dimmable_lights_are_not_sent_invalid_commands() {
        for (state_value, error) in [
            (
                state("unavailable", json!(["brightness"])),
                Error::Unavailable,
            ),
            (state("on", json!(["onoff"])), Error::UnsupportedBrightness),
        ] {
            let (url, s) = server(vec![(200, state_value)]);
            assert_eq!(
                HomeAssistant::new(&url, "test-secret")
                    .unwrap()
                    .command("light.test", Command::Brightness(50)),
                Err(error)
            );
            assert_eq!(s.join().unwrap().len(), 1);
        }
    }
    #[test]
    fn errors_are_classified_without_leaking_server_body_or_token() {
        for (code, error) in [
            (401, Error::Authentication),
            (404, Error::Missing),
            (500, Error::Status(500)),
        ] {
            let (url, s) = server(vec![(code, json!({"error":"test-secret"}))]);
            let client = HomeAssistant::new(&url, "test-secret").unwrap();
            assert!(!format!("{client:?}").contains("test-secret"));
            assert_eq!(client.lights(), Err(error));
            s.join().unwrap();
        }
    }
    #[test]
    fn rejects_unsafe_configuration_and_non_light_targets() {
        for url in [
            "ftp://host",
            "http://user:password@host",
            "http://host?token=secret",
            "http://host/#fragment",
        ] {
            assert!(HomeAssistant::new(url, "test-secret").is_err());
        }
        assert!(HomeAssistant::new("http://localhost", "secret\r\nInjected: value").is_err());
        let client = HomeAssistant::new("http://127.0.0.1:1", "test-secret").unwrap();
        assert_eq!(client.light("switch.test"), Err(Error::Configuration));
        assert_eq!(client.light("light.a/../all"), Err(Error::Configuration));
        assert_eq!(
            client.command("light.test", Command::Brightness(101)),
            Err(Error::InvalidBrightness)
        );
    }
    #[test]
    fn mismatched_entity_and_malformed_discovery_are_rejected() {
        let (url, s) = server(vec![
            (200, json!({"not":"a list"})),
            (
                200,
                json!({"entity_id":"light.other","state":"on","attributes":{}}),
            ),
        ]);
        let client = HomeAssistant::new(&url, "test-secret").unwrap();
        assert_eq!(client.lights(), Err(Error::Response));
        assert_eq!(client.light("light.test"), Err(Error::Response));
        s.join().unwrap();
    }
    #[test]
    fn timeouts_do_not_leave_callers_waiting_indefinitely() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let client = HomeAssistant::with_timeout(
            &format!("http://{}", server.server_addr()),
            "test-secret",
            Duration::from_millis(100),
        )
        .unwrap();
        let start = std::time::Instant::now();
        assert_eq!(client.lights(), Err(Error::Transport));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
