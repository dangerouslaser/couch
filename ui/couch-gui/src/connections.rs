//! Connection-scoped credentials and state. Resource IDs may repeat on different servers.
use crate::home;
use couch_model::{Config, Provider};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
pub fn config() -> Option<Arc<Config>> {
    crate::config_snapshot::current().map(|s| s.config.clone())
}

pub fn file(id: &str, prefix: &str) -> PathBuf {
    if id.is_empty() {
        home::path(&format!("{prefix}-connection.json"))
    } else {
        home::path(&format!("connections/{id}/{prefix}-connection.json"))
    }
}
pub fn split(resource: &str) -> (&str, &str) {
    resource.split_once('/').unwrap_or(("", resource))
}
fn valid(id: &str, provider: Provider) -> bool {
    if id.is_empty() {
        return true;
    }
    config().is_some_and(|c| {
        c.connections
            .iter()
            .any(|v| v.id.as_str() == id && v.provider == provider)
    })
}
fn ids(provider: Provider) -> Vec<String> {
    let ids: Vec<_> = config()
        .map(|c| {
            c.connections
                .iter()
                .filter(|c| c.provider == provider)
                .map(|c| c.id.to_string())
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        vec![String::new()]
    } else {
        ids
    }
}
pub fn ha(resource: &str) -> Result<(couch_ha::HomeAssistant, String), String> {
    let (id, raw) = split(resource);
    if !valid(id, Provider::HomeAssistant) {
        return Err("Home Assistant connection was removed".into());
    }
    let client = couch_ha::settings::Settings::load(&file(id, "ha"))
        .and_then(|s| s.client())
        .map_err(|e| e.to_string())?;
    Ok((client, raw.into()))
}
pub fn ha_lights() -> Vec<couch_ha::Light> {
    ids(Provider::HomeAssistant)
        .into_iter()
        .flat_map(|id| {
            couch_ha::settings::Settings::load(&file(&id, "ha"))
                .and_then(|s| s.client())
                .and_then(|c| c.lights())
                .unwrap_or_default()
                .into_iter()
                .map(move |mut l| {
                    if !id.is_empty() {
                        l.entity_id = format!("{id}/{}", l.entity_id);
                    }
                    l
                })
        })
        .collect()
}
/// Fetch all room-supported HA entities once per connection. Preserve the
/// connection prefix because entity IDs are only unique within one server.
pub fn ha_room_states() -> Vec<crate::lights::DeviceState> {
    use crate::lights::DeviceState;
    let mut result = Vec::new();
    for id in ids(Provider::HomeAssistant) {
        let Ok(client) = couch_ha::settings::Settings::load(&file(&id, "ha"))
            .and_then(|settings| settings.client())
        else {
            continue;
        };
        let Ok(entities) = client.entities() else {
            continue;
        };
        let mut states = Vec::new();
        states.extend(entities.lights.into_iter().map(DeviceState::Light));
        states.extend(entities.covers.into_iter().map(DeviceState::Cover));
        states.extend(entities.climates.into_iter().map(DeviceState::Climate));
        for mut state in states {
            if !id.is_empty() {
                state.set_id(format!("{id}/{}", state.id()));
            }
            result.push(state);
        }
    }
    result
}
#[derive(Default)]
pub struct HueFleet {
    clients: Mutex<HashMap<String, Arc<couch_hue::live::Live>>>,
}
impl HueFleet {
    fn get(&self, id: &str) -> Result<Arc<couch_hue::live::Live>, String> {
        if !valid(id, Provider::Hue) {
            return Err("Hue connection was removed".into());
        }
        let mut clients = self.clients.lock().unwrap();
        Ok(clients
            .entry(id.into())
            .or_insert_with(|| Arc::new(couch_hue::live::Live::new(file(id, "hue"))))
            .clone())
    }
    pub fn lights(&self) -> Result<Vec<couch_ha::Light>, String> {
        let ids = ids(Provider::Hue);
        self.clients
            .lock()
            .unwrap()
            .retain(|id, _| ids.contains(id));
        let mut lights = vec![];
        for id in ids {
            if let Ok(client) = self.get(&id) {
                for mut light in client.lights().unwrap_or_default() {
                    if !id.is_empty() {
                        light.entity_id = format!("{id}/{}", light.entity_id);
                    }
                    lights.push(light);
                }
            }
        }
        Ok(lights)
    }
    pub fn toggle(&self, resource: &str) -> Result<couch_ha::Light, String> {
        let (id, raw) = split(resource);
        self.get(id)?.toggle(raw).map_err(|e| e.to_string())
    }
    pub fn brightness(&self, resource: &str, level: u8) -> Result<couch_ha::Light, String> {
        let (id, raw) = split(resource);
        self.get(id)?
            .brightness(raw, level)
            .map_err(|e| e.to_string())
    }
    pub fn reset(&self) {
        for client in self.clients.lock().unwrap().values() {
            client.reset();
        }
    }
}
