//! Named connections are shared by devices; credentials stay on the remote.
use crate::{Config, Id, Integration};
use alloc::string::String;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: Id,
    pub name: String,
    pub provider: Provider,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Provider {
    Kodi { host: String, port: u16 },
    HomeAssistant,
    Hue,
    Ir,
}
impl Provider {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Kodi { .. } => "kodi",
            Self::HomeAssistant => "home-assistant",
            Self::Hue => "hue",
            Self::Ir => "ir",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Kodi { .. } => "Kodi",
            Self::HomeAssistant => "Home Assistant",
            Self::Hue => "Philips Hue",
            Self::Ir => "Infrared",
        }
    }
}
impl Config {
    pub fn connection(&self, id: &Id) -> Option<&Connection> {
        self.connections.iter().find(|c| &c.id == id)
    }
    /// Legacy inline integrations remain readable. New devices refer to a
    /// connection, so changing a Kodi address updates every referring device.
    pub fn resolve_integration(&self, integration: &Integration) -> Option<Integration> {
        let Integration::Connection {
            connection_id,
            resource_id,
        } = integration
        else {
            return Some(integration.clone());
        };
        Some(match &self.connection(connection_id)?.provider {
            Provider::Kodi { host, port } => Integration::Kodi {
                host: host.clone(),
                port: *port,
            },
            Provider::HomeAssistant => Integration::HomeAssistant {
                entity_id: resource_id.clone(),
            },
            Provider::Hue => Integration::Hue {
                light_id: resource_id.clone(),
            },
            Provider::Ir => Integration::Ir {
                codeset: resource_id.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Device, DeviceKind, Room};
    use alloc::vec;
    #[test]
    fn shared_connection_updates_devices_and_cannot_be_deleted_in_use() {
        let mut c = Config::default();
        c.connections.push(Connection {
            id: "player".into(),
            name: "Player".into(),
            provider: Provider::Kodi {
                host: "old.local".into(),
                port: 9090,
            },
        });
        let integration = Integration::Connection {
            connection_id: "player".into(),
            resource_id: String::new(),
        };
        let mut room = Room {
            id: "room".into(),
            name: "Room".into(),
            icon: None,
            devices: vec![],
        };
        room.devices = vec![Device::new("tv".into(), "Player", DeviceKind::MediaPlayer)
            .with_integration(integration.clone())];
        c.rooms.push(room);
        assert!(c.validate().is_ok());
        c.connections[0].provider = Provider::Kodi {
            host: "new.local".into(),
            port: 9090,
        };
        assert_eq!(
            c.resolve_integration(&integration),
            Some(Integration::Kodi {
                host: "new.local".into(),
                port: 9090
            })
        );
        c.connections.clear();
        assert!(c.validate().is_err());
    }
    #[test]
    fn old_config_remains_readable_without_connections() {
        let c: Config = serde_json::from_str(r#"{"schema_version":1,"rooms":[]}"#).unwrap();
        assert!(c.connections.is_empty());
    }
}
