use alloc::string::String;
use serde::{Deserialize, Serialize};
/// Clock presentation and idle behavior; empty timezone follows system time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSettings {
    pub timezone: String,
    pub clock_24h: bool,
    pub dock_clock: bool,
}
impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            timezone: String::new(),
            clock_24h: true,
            dock_clock: true,
        }
    }
}
