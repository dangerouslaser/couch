use alloc::string::String;
use serde::{Deserialize, Serialize};
/// Clock presentation and idle behavior; empty timezone follows system time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSettings {
    pub timezone: String,
    pub clock_24h: bool,
    pub dock_clock: bool,
    pub wake_on_lift: bool,
}
impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            timezone: String::new(),
            clock_24h: true,
            dock_clock: true,
            wake_on_lift: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_settings_enable_lift_and_preserve_explicit_opt_out() {
        let old: RemoteSettings = serde_json::from_str(r#"{"clock_24h":false}"#).unwrap();
        assert!(old.wake_on_lift);
        assert!(!old.clock_24h);
        let disabled: RemoteSettings = serde_json::from_str(r#"{"wake_on_lift":false}"#).unwrap();
        assert!(!disabled.wake_on_lift);
    }
}
