//! Standard HA cover/climate state and service contracts. Temperature values use
//! HA's configured display unit, as do the REST set_temperature service values.
use crate::{Error, HomeAssistant, Light, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn valid_id(id: &str, domain: &str) -> bool {
    id.strip_prefix(domain)
        .and_then(|s| s.strip_prefix('.'))
        .is_some_and(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|v| v.is_finite())
}
fn modes(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| valid_mode(s))
        .map(str::to_owned)
        .collect()
}
fn valid_mode(s: &str) -> bool {
    matches!(
        s,
        "off" | "heat" | "cool" | "heat_cool" | "auto" | "dry" | "fan_only"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cover {
    pub entity_id: String,
    pub name: String,
    pub state: Option<String>,
    /// 0 is fully closed; 100 is fully open. Unknown is never inferred as zero.
    pub position_percent: Option<u8>,
    pub can_open: bool,
    pub can_close: bool,
    pub can_set_position: bool,
    pub can_stop: bool,
}
impl Cover {
    pub fn from_state(v: &Value) -> Option<Self> {
        let id = v["entity_id"].as_str()?;
        if !valid_id(id, "cover") {
            return None;
        }
        let state = v["state"]
            .as_str()
            .filter(|s| matches!(*s, "open" | "closed" | "opening" | "closing"))
            .map(str::to_owned);
        let a = &v["attributes"];
        // CoverEntityFeature in homeassistant/components/cover/const.py.
        let flags = a["supported_features"].as_u64().unwrap_or(0);
        let position = if state.is_some() {
            a["current_position"]
                .as_u64()
                .filter(|p| *p <= 100)
                .map(|p| p as u8)
        } else {
            None
        };
        Some(Self {
            entity_id: id.into(),
            name: a["friendly_name"].as_str().unwrap_or(id).into(),
            state,
            position_percent: position,
            can_open: flags & 1 != 0,
            can_close: flags & 2 != 0,
            can_set_position: flags & 4 != 0,
            can_stop: flags & 8 != 0,
        })
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum CoverCommand {
    Open,
    Close,
    Toggle,
    Position(u8),
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Climate {
    pub entity_id: String,
    pub name: String,
    pub available: bool,
    pub hvac_mode: Option<String>,
    pub hvac_modes: Vec<String>,
    pub hvac_action: Option<String>,
    pub current_temperature: Option<f64>,
    pub target_temperature: Option<f64>,
    pub target_temperature_low: Option<f64>,
    pub target_temperature_high: Option<f64>,
    pub min_temperature: f64,
    pub max_temperature: f64,
    pub temperature_step: f64,
    pub temperature_unit: String,
    pub supports_target_temperature: bool,
    pub supports_target_range: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ClimateCommand {
    Temperature(f64),
    TemperatureRange { low: f64, high: f64 },
    HvacMode(String),
}
impl Climate {
    pub fn from_state(v: &Value, unit: &str) -> Option<Self> {
        let id = v["entity_id"].as_str()?;
        if !valid_id(id, "climate") || !matches!(unit, "°C" | "°F") {
            return None;
        }
        let a = &v["attributes"];
        let hvac_mode = v["state"]
            .as_str()
            .filter(|s| valid_mode(s))
            .map(str::to_owned);
        let available = hvac_mode.is_some();
        let flags = a["supported_features"].as_u64().unwrap_or(0);
        // HA normally includes both limits. Fail closed for malformed/missing
        // capabilities: display the entity but do not invent writable limits.
        let min = number(&a["min_temp"]);
        let max = number(&a["max_temp"]);
        let limits_valid = min.zip(max).is_some_and(|(lo, hi)| lo < hi);
        let read = |key: &str| if available { number(&a[key]) } else { None };
        Some(Self {
            entity_id: id.into(),
            name: a["friendly_name"].as_str().unwrap_or(id).into(),
            available,
            hvac_mode,
            hvac_modes: modes(&a["hvac_modes"]),
            hvac_action: if available {
                a["hvac_action"].as_str().map(str::to_owned)
            } else {
                None
            },
            current_temperature: read("current_temperature"),
            target_temperature: read("temperature"),
            target_temperature_low: read("target_temp_low"),
            target_temperature_high: read("target_temp_high"),
            min_temperature: min.unwrap_or(0.0),
            max_temperature: max.unwrap_or(0.0),
            temperature_step: number(&a["target_temp_step"])
                .filter(|s| *s > 0.0)
                .unwrap_or(if unit == "°F" { 1.0 } else { 0.5 }),
            temperature_unit: unit.into(),
            supports_target_temperature: limits_valid && flags & 1 != 0,
            supports_target_range: limits_valid && flags & 2 != 0,
        })
    }
    fn range_mode(&self) -> bool {
        self.hvac_mode.as_deref() == Some("heat_cool")
            || (!self.supports_target_temperature && self.supports_target_range)
    }
    fn validate(&self, cmd: &ClimateCommand) -> Result<()> {
        if !self.available {
            return Err(Error::Unavailable);
        }
        let valid_temp =
            |t: f64| t.is_finite() && t >= self.min_temperature && t <= self.max_temperature;
        match cmd {
            ClimateCommand::Temperature(t) => {
                if !self.supports_target_temperature || self.range_mode() {
                    return Err(Error::UnsupportedOperation);
                }
                if !valid_temp(*t) {
                    return Err(Error::InvalidTemperature);
                }
            }
            ClimateCommand::TemperatureRange { low, high } => {
                if !self.supports_target_range || !self.range_mode() {
                    return Err(Error::UnsupportedOperation);
                }
                if !valid_temp(*low) || !valid_temp(*high) || low > high {
                    return Err(Error::InvalidTemperature);
                }
            }
            ClimateCommand::HvacMode(mode) => {
                if !self.hvac_modes.contains(mode) {
                    return Err(Error::UnsupportedOperation);
                }
            }
        }
        Ok(())
    }
    /// Shift the existing target by advertised increments. A pending command can
    /// be supplied for coalescing repeated keys while HA confirms the prior one.
    /// In range mode shift both limits together, preserving the heating/cooling gap.
    pub fn adjusted_target(
        &self,
        delta: i32,
        previous: Option<&ClimateCommand>,
    ) -> Result<ClimateCommand> {
        if !self.available {
            return Err(Error::Unavailable);
        }
        if matches!(
            self.hvac_mode.as_deref(),
            Some("off" | "auto" | "dry" | "fan_only") | None
        ) {
            return Err(Error::UnsupportedOperation);
        }
        let amount = f64::from(delta) * self.temperature_step;
        let command = if self.range_mode() {
            let (low, high) = match previous {
                Some(ClimateCommand::TemperatureRange { low, high }) => (*low, *high),
                _ => (
                    self.target_temperature_low.ok_or(Error::Unavailable)?,
                    self.target_temperature_high.ok_or(Error::Unavailable)?,
                ),
            };
            self.validate(&ClimateCommand::TemperatureRange { low, high })?;
            let shift = amount.clamp(self.min_temperature - low, self.max_temperature - high);
            ClimateCommand::TemperatureRange {
                low: low + shift,
                high: high + shift,
            }
        } else {
            let target = match previous {
                Some(ClimateCommand::Temperature(t)) => *t,
                _ => self.target_temperature.ok_or(Error::Unavailable)?,
            };
            self.validate(&ClimateCommand::Temperature(target))?;
            ClimateCommand::Temperature(
                (target + amount).clamp(self.min_temperature, self.max_temperature),
            )
        };
        self.validate(&command)?;
        Ok(command)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entities {
    pub lights: Vec<Light>,
    pub covers: Vec<Cover>,
    pub climates: Vec<Climate>,
}
impl HomeAssistant {
    fn temperature_unit(&self) -> Result<String> {
        if let Some((when, unit)) = self
            .temperature_unit
            .lock()
            .map_err(|_| Error::Response)?
            .as_ref()
        {
            if when.elapsed() < Duration::from_secs(300) {
                return Ok(unit.clone());
            }
        }
        let config = self.get("/api/config")?;
        let unit = config["unit_system"]["temperature"]
            .as_str()
            .filter(|u| matches!(*u, "°C" | "°F"))
            .ok_or(Error::Response)?
            .to_owned();
        *self.temperature_unit.lock().map_err(|_| Error::Response)? =
            Some((Instant::now(), unit.clone()));
        Ok(unit)
    }
    pub fn covers(&self) -> Result<Vec<Cover>> {
        let values = self.get("/api/states")?;
        let mut covers: Vec<_> = values
            .as_array()
            .ok_or(Error::Response)?
            .iter()
            .filter_map(Cover::from_state)
            .collect();
        covers.sort_by(|a, b| a.name.cmp(&b.name).then(a.entity_id.cmp(&b.entity_id)));
        Ok(covers)
    }
    pub fn cover(&self, id: &str) -> Result<Cover> {
        if !valid_id(id, "cover") {
            return Err(Error::Configuration);
        }
        Cover::from_state(&self.get(&format!("/api/states/{id}"))?)
            .filter(|c| c.entity_id == id)
            .ok_or(Error::Response)
    }
    pub fn climates(&self) -> Result<Vec<Climate>> {
        let values = self.get("/api/states")?;
        let values = values.as_array().ok_or(Error::Response)?;
        if !values.iter().any(|v| {
            v["entity_id"]
                .as_str()
                .is_some_and(|s| valid_id(s, "climate"))
        }) {
            return Ok(Vec::new());
        }
        let unit = self.temperature_unit()?;
        let mut climates: Vec<_> = values
            .iter()
            .filter_map(|v| Climate::from_state(v, &unit))
            .collect();
        climates.sort_by(|a, b| a.name.cmp(&b.name).then(a.entity_id.cmp(&b.entity_id)));
        Ok(climates)
    }
    pub fn climate(&self, id: &str) -> Result<Climate> {
        if !valid_id(id, "climate") {
            return Err(Error::Configuration);
        }
        let value = self.get(&format!("/api/states/{id}"))?;
        if value["entity_id"].as_str() != Some(id) {
            return Err(Error::Response);
        }
        Climate::from_state(&value, &self.temperature_unit()?).ok_or(Error::Response)
    }
    pub fn entities(&self) -> Result<Entities> {
        let values = self.get("/api/states")?;
        let values = values.as_array().ok_or(Error::Response)?;
        let unit = if values.iter().any(|v| {
            v["entity_id"]
                .as_str()
                .is_some_and(|s| valid_id(s, "climate"))
        }) {
            self.temperature_unit()?
        } else {
            "°C".into()
        };
        let mut result = Entities {
            lights: values.iter().filter_map(Light::from_state).collect(),
            covers: values.iter().filter_map(Cover::from_state).collect(),
            climates: values
                .iter()
                .filter_map(|v| Climate::from_state(v, &unit))
                .collect(),
        };
        result
            .lights
            .sort_by(|a, b| a.name.cmp(&b.name).then(a.entity_id.cmp(&b.entity_id)));
        result
            .covers
            .sort_by(|a, b| a.name.cmp(&b.name).then(a.entity_id.cmp(&b.entity_id)));
        result
            .climates
            .sort_by(|a, b| a.name.cmp(&b.name).then(a.entity_id.cmp(&b.entity_id)));
        Ok(result)
    }
    pub fn cover_command(&self, id: &str, cmd: CoverCommand) -> Result<()> {
        if matches!(cmd,CoverCommand::Position(p) if p>100) {
            return Err(Error::InvalidPosition);
        }
        let cover = self.cover(id)?;
        let state = cover.state.as_deref().ok_or(Error::Unavailable)?;
        let cmd = if cmd == CoverCommand::Toggle {
            if matches!(state, "closed" | "closing") {
                CoverCommand::Open
            } else {
                CoverCommand::Close
            }
        } else {
            cmd
        };
        let (service, supported, data) = match cmd {
            CoverCommand::Open => ("open_cover", cover.can_open, json!({"entity_id":id})),
            CoverCommand::Close => ("close_cover", cover.can_close, json!({"entity_id":id})),
            CoverCommand::Stop => ("stop_cover", cover.can_stop, json!({"entity_id":id})),
            CoverCommand::Position(p) => (
                "set_cover_position",
                cover.can_set_position,
                json!({"entity_id":id,"position":p}),
            ),
            CoverCommand::Toggle => unreachable!(),
        };
        if !supported {
            return Err(Error::UnsupportedOperation);
        }
        self.service("cover", service, data)
    }
    pub fn climate_command(&self, id: &str, cmd: ClimateCommand) -> Result<()> {
        let climate = self.climate(id)?;
        climate.validate(&cmd)?;
        let (service, data) = match cmd {
            ClimateCommand::Temperature(t) => {
                ("set_temperature", json!({"entity_id":id,"temperature":t}))
            }
            ClimateCommand::TemperatureRange { low, high } => (
                "set_temperature",
                json!({"entity_id":id,"target_temp_low":low,"target_temp_high":high}),
            ),
            ClimateCommand::HvacMode(mode) => {
                ("set_hvac_mode", json!({"entity_id":id,"hvac_mode":mode}))
            }
        };
        self.service("climate", service, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::server;
    fn cover(state: &str, flags: u64) -> Value {
        json!({"entity_id":"cover.test","state":state,"attributes":{"supported_features":flags,"current_position":37}})
    }
    fn climate(mode: &str, flags: u64) -> Value {
        json!({"entity_id":"climate.test","state":mode,"attributes":{"supported_features":flags,"hvac_modes":["off","heat","cool","heat_cool"],"min_temp":10,"max_temp":30,"target_temp_step":0.5,"temperature":21.5,"target_temp_low":19,"target_temp_high":24,"current_temperature":20.3}})
    }
    fn config(unit: &str) -> Value {
        json!({"unit_system":{"temperature":unit}})
    }
    #[test]
    fn cover_services_preserve_open_position_direction_and_toggle_motion() {
        for (state, cmd, path, payload) in [
            (
                "closed",
                CoverCommand::Toggle,
                "open_cover",
                json!({"entity_id":"cover.test"}),
            ),
            (
                "closing",
                CoverCommand::Toggle,
                "open_cover",
                json!({"entity_id":"cover.test"}),
            ),
            (
                "opening",
                CoverCommand::Toggle,
                "close_cover",
                json!({"entity_id":"cover.test"}),
            ),
            (
                "open",
                CoverCommand::Position(75),
                "set_cover_position",
                json!({"entity_id":"cover.test","position":75}),
            ),
            (
                "opening",
                CoverCommand::Stop,
                "stop_cover",
                json!({"entity_id":"cover.test"}),
            ),
        ] {
            let (url, s) = server(vec![(200, cover(state, 15)), (200, json!([]))]);
            HomeAssistant::new(&url, "test-secret")
                .unwrap()
                .cover_command("cover.test", cmd)
                .unwrap();
            assert_eq!(
                s.join().unwrap()[1],
                (
                    "POST".into(),
                    format!("/api/services/cover/{path}"),
                    payload
                )
            );
        }
    }
    #[test]
    fn unavailable_or_unsupported_cover_never_posts() {
        for (state, flags, error) in [
            ("unavailable", 15, Error::Unavailable),
            ("open", 3, Error::UnsupportedOperation),
        ] {
            let (url, s) = server(vec![(200, cover(state, flags))]);
            assert_eq!(
                HomeAssistant::new(&url, "test-secret")
                    .unwrap()
                    .cover_command("cover.test", CoverCommand::Position(20)),
                Err(error)
            );
            assert_eq!(s.join().unwrap().len(), 1);
        }
        let c = Cover::from_state(&cover("unavailable", 15)).unwrap();
        assert_eq!(c.position_percent, None);
    }
    #[test]
    fn discovery_uses_one_state_snapshot_and_reads_configured_unit() {
        let (url, s) = server(vec![
            (
                200,
                json!([cover("closed",3),climate("heat",1),{"entity_id":"sensor.other","state":"12"}]),
            ),
            (200, config("°F")),
        ]);
        let entities = HomeAssistant::new(&url, "test-secret")
            .unwrap()
            .entities()
            .unwrap();
        assert_eq!(entities.covers.len(), 1);
        assert_eq!(entities.climates[0].temperature_unit, "°F");
        assert_eq!(entities.climates[0].current_temperature, Some(20.3));
        assert_eq!(s.join().unwrap().len(), 2);
    }
    #[test]
    fn climate_services_emit_correct_target_range_and_mode_payloads() {
        for (mode, flags, cmd, service, data) in [
            (
                "heat",
                1,
                ClimateCommand::Temperature(22.0),
                "set_temperature",
                json!({"entity_id":"climate.test","temperature":22.0}),
            ),
            (
                "heat_cool",
                2,
                ClimateCommand::TemperatureRange {
                    low: 19.5,
                    high: 24.5,
                },
                "set_temperature",
                json!({"entity_id":"climate.test","target_temp_low":19.5,"target_temp_high":24.5}),
            ),
            (
                "off",
                1,
                ClimateCommand::HvacMode("heat".into()),
                "set_hvac_mode",
                json!({"entity_id":"climate.test","hvac_mode":"heat"}),
            ),
        ] {
            let (url, s) = server(vec![
                (200, climate(mode, flags)),
                (200, config("°C")),
                (200, json!([])),
            ]);
            HomeAssistant::new(&url, "test-secret")
                .unwrap()
                .climate_command("climate.test", cmd)
                .unwrap();
            assert_eq!(
                s.join().unwrap()[2],
                (
                    "POST".into(),
                    format!("/api/services/climate/{service}"),
                    data
                )
            );
        }
    }
    #[test]
    fn invalid_climate_commands_do_not_post() {
        for (mode, flags, cmd, error) in [
            (
                "unavailable",
                1,
                ClimateCommand::Temperature(22.0),
                Error::Unavailable,
            ),
            (
                "heat",
                0,
                ClimateCommand::Temperature(22.0),
                Error::UnsupportedOperation,
            ),
            (
                "heat_cool",
                3,
                ClimateCommand::Temperature(22.0),
                Error::UnsupportedOperation,
            ),
            (
                "heat",
                1,
                ClimateCommand::Temperature(f64::NAN),
                Error::InvalidTemperature,
            ),
            (
                "heat",
                1,
                ClimateCommand::Temperature(31.0),
                Error::InvalidTemperature,
            ),
            (
                "heat_cool",
                2,
                ClimateCommand::TemperatureRange {
                    low: 25.0,
                    high: 20.0,
                },
                Error::InvalidTemperature,
            ),
            (
                "heat",
                1,
                ClimateCommand::HvacMode("bogus".into()),
                Error::UnsupportedOperation,
            ),
        ] {
            let (url, s) = server(vec![(200, climate(mode, flags)), (200, config("°C"))]);
            assert_eq!(
                HomeAssistant::new(&url, "test-secret")
                    .unwrap()
                    .climate_command("climate.test", cmd),
                Err(error)
            );
            assert_eq!(s.join().unwrap().len(), 2);
        }
    }
    #[test]
    fn volume_adjustment_honors_step_pending_value_bounds_and_range_gap() {
        let single = Climate::from_state(&climate("heat", 1), "°C").unwrap();
        assert_eq!(
            single.adjusted_target(1, None).unwrap(),
            ClimateCommand::Temperature(22.0)
        );
        assert_eq!(
            single
                .adjusted_target(1, Some(&ClimateCommand::Temperature(22.0)))
                .unwrap(),
            ClimateCommand::Temperature(22.5)
        );
        assert_eq!(
            single.adjusted_target(-100, None).unwrap(),
            ClimateCommand::Temperature(10.0)
        );
        let range = Climate::from_state(&climate("heat_cool", 2), "°C").unwrap();
        assert_eq!(
            range.adjusted_target(1, None).unwrap(),
            ClimateCommand::TemperatureRange {
                low: 19.5,
                high: 24.5
            }
        );
        assert_eq!(
            range.adjusted_target(100, None).unwrap(),
            ClimateCommand::TemperatureRange {
                low: 25.0,
                high: 30.0
            }
        );
        assert_eq!(
            range.adjusted_target(-100, None).unwrap(),
            ClimateCommand::TemperatureRange {
                low: 10.0,
                high: 15.0
            }
        );
        let off = Climate::from_state(&climate("off", 1), "°C").unwrap();
        assert_eq!(
            off.adjusted_target(1, None),
            Err(Error::UnsupportedOperation)
        );
        let mut missing = climate("heat", 1);
        missing["attributes"]["temperature"] = Value::Null;
        assert_eq!(
            Climate::from_state(&missing, "°C")
                .unwrap()
                .adjusted_target(1, None),
            Err(Error::Unavailable)
        );
    }
    #[test]
    fn missing_limits_fail_closed_and_unavailable_values_do_not_look_current() {
        let mut missing = climate("heat", 1);
        missing["attributes"]["min_temp"] = Value::Null;
        let parsed = Climate::from_state(&missing, "°C").unwrap();
        assert!(!parsed.supports_target_temperature);
        let parsed = Climate::from_state(&climate("unknown", 3), "°C").unwrap();
        assert!(!parsed.available);
        assert_eq!(parsed.current_temperature, None);
        assert_eq!(parsed.target_temperature, None);
    }
    #[test]
    fn unit_cache_avoids_repeated_config_fetches() {
        let (url, s) = server(vec![
            (200, climate("heat", 1)),
            (200, config("°C")),
            (200, climate("heat", 1)),
        ]);
        let client = HomeAssistant::new(&url, "test-secret").unwrap();
        client.climate("climate.test").unwrap();
        client.climate("climate.test").unwrap();
        assert_eq!(s.join().unwrap().len(), 3);
    }
    #[test]
    fn unsafe_entities_mismatches_and_unknown_units_are_rejected() {
        let client = HomeAssistant::new("http://127.0.0.1:1", "test-secret").unwrap();
        assert_eq!(client.cover("cover.x/../all"), Err(Error::Configuration));
        assert_eq!(client.climate("light.test"), Err(Error::Configuration));
        assert_eq!(
            client.cover_command("cover.test", CoverCommand::Position(101)),
            Err(Error::InvalidPosition)
        );
        let (url, s) = server(vec![(
            200,
            json!({"entity_id":"climate.other","state":"heat"}),
        )]);
        assert_eq!(
            HomeAssistant::new(&url, "test-secret")
                .unwrap()
                .climate("climate.test"),
            Err(Error::Response)
        );
        s.join().unwrap();
        let (url, s) = server(vec![(200, climate("heat", 1)), (200, config("unknown"))]);
        assert_eq!(
            HomeAssistant::new(&url, "test-secret")
                .unwrap()
                .climate("climate.test"),
            Err(Error::Response)
        );
        s.join().unwrap();
    }
}
