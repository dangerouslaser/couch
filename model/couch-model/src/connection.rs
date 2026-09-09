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
    WebOs,
    Ir,
}
impl Provider {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Kodi { .. } => "kodi",
            Self::HomeAssistant => "home-assistant",
            Self::Hue => "hue",
            Self::WebOs => "web-os",
            Self::Ir => "ir",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Kodi { .. } => "Kodi",
            Self::HomeAssistant => "Home Assistant",
            Self::Hue => "Philips Hue",
            Self::WebOs => "LG webOS",
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
    /// Resolved Hue/HA IDs are runtime cache keys (connection/resource); strip
    /// the connection prefix before calling the upstream API. Stored IDs stay raw.
    pub fn resolve_integration(&self, integration: &Integration) -> Option<Integration> {
        let Integration::Connection {
            connection_id,
            resource_id,
        } = integration
        else {
            let legacy = match integration {Integration::Hue{light_id}=>Some(("hue",light_id)),Integration::HomeAssistant{entity_id}=>Some(("home-assistant",entity_id)),_=>None};
            if let Some((kind,resource))=legacy {
                if let Some(c)=self.connections.iter().find(|c|c.provider.kind()==kind) {
                    return self.resolve_integration(&Integration::Connection{connection_id:c.id.clone(),resource_id:resource.clone()});
                }
            }
            return Some(integration.clone());
        };
        Some(match &self.connection(connection_id)?.provider {
            Provider::Kodi { host, port } => Integration::Kodi {
                host: host.clone(),
                port: *port,
            },
            Provider::HomeAssistant => Integration::HomeAssistant {
                entity_id: alloc::format!("{connection_id}/{resource_id}"),
            },
            Provider::Hue => Integration::Hue {
                light_id: alloc::format!("{connection_id}/{resource_id}"),
            },
            Provider::WebOs => Integration::WebOs,
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
    fn hue_scene_room_assignments_validate_and_survive_round_trip() {
        let mut c = Config::default();
        c.connections.push(Connection {
            id: "hue".into(),
            name: "Hue".into(),
            provider: Provider::Hue,
        });
        c.rooms.push(Room {
            id: "office".into(),
            name: "Office".into(),
            icon: None,
            devices: vec![],
        });
        c.scenes.push(crate::Scene {
            id: "relax".into(),
            name: "Relax".into(),
            icon: None,
            steps: vec![],
            rooms: vec!["office".into()],
            hue: Some(crate::HueScene {
                connection_id: "hue".into(),
                scene_id: "00000000-0000-0000-0000-000000000001".into(),
            }),
        });
        assert!(c.validate().is_ok());
        let saved = serde_json::to_string(&c).unwrap();
        let restored: Config = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored, c);
        c.connections.clear();
        assert!(c.validate().is_err());
        let mut c = restored;
        c.remove_room(&"office".into());
        assert!(c.scenes[0].rooms.is_empty());
        assert!(c.validate().is_ok());
    }
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
    fn webos_connection_round_trips_and_resolves_room_tv() {
        let mut config = Config::default();
        config.connections.push(Connection {id:"lg".into(),name:"LG TV".into(),provider:Provider::WebOs});
        let integration = Integration::Connection {connection_id:"lg".into(),resource_id:String::new()};
        config.rooms.push(Room {id:"office".into(),name:"Office".into(),icon:None,devices:vec![Device::new("tv".into(),"LG TV",DeviceKind::Tv).with_integration(integration.clone())]});
        assert!(config.validate().is_ok());
        assert_eq!(config.resolve_integration(&integration),Some(Integration::WebOs));
        let saved=serde_json::to_string(&config).unwrap();
        assert!(saved.contains("web-os"));
        assert_eq!(serde_json::from_str::<Config>(&saved).unwrap(),config);
        config.connections.push(Connection {id:"second".into(),name:"Second TV".into(),provider:Provider::WebOs});
        assert!(config.validate().is_ok());
    }
    #[test]
    fn identical_resources_on_different_servers_remain_distinct() {
        let mut c=Config::default();
        for (id,provider) in [("ha-a",Provider::HomeAssistant),("ha-b",Provider::HomeAssistant),("hue-a",Provider::Hue),("hue-b",Provider::Hue)] {
            c.connections.push(Connection{id:id.into(),name:id.into(),provider});
        }
        assert!(c.validate().is_ok());
        for (a,b,resource) in [("ha-a","ha-b","light.same"),("hue-a","hue-b","00000000-0000-0000-0000-000000000001")] {
            let left=c.resolve_integration(&Integration::Connection{connection_id:a.into(),resource_id:resource.into()});
            let right=c.resolve_integration(&Integration::Connection{connection_id:b.into(),resource_id:resource.into()});
            assert_ne!(left,right);
        }
        c.connections[0].id="../escape".into();assert!(c.validate().is_err());
    }
    #[test]
    fn built_in_ir_is_shared_but_each_device_keeps_its_own_codeset() {
        let mut c=Config::default();
        c.connections.push(Connection{id:"ir".into(),name:"Built-in IR".into(),provider:Provider::Ir});
        c.rooms.push(Room{id:"room".into(),name:"Room".into(),icon:None,devices:vec![
            Device::new("tv".into(),"TV",DeviceKind::Tv).with_integration(Integration::Connection{connection_id:"ir".into(),resource_id:"lg-tv".into()}),
            Device::new("amp".into(),"Amplifier",DeviceKind::Speaker).with_integration(Integration::Connection{connection_id:"ir".into(),resource_id:"denon".into()})]});
        assert!(c.validate().is_ok());
        c.connections.push(Connection{id:"another-ir".into(),name:"Duplicate blaster".into(),provider:Provider::Ir});assert!(c.validate().is_err());
    }
    #[test]
    fn old_config_remains_readable_without_connections() {
        let c: Config = serde_json::from_str(r#"{"schema_version":1,"rooms":[]}"#).unwrap();
        assert!(c.connections.is_empty());
    }
}
