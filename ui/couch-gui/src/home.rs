//! Saved house configuration projected into the device's existing room rows.
use crate::{LiveActivity, RoomRow, SceneCell};
use couch_model::{Config, Id};
use std::path::PathBuf;
pub struct Area {
    pub name: String,
    pub activities: Vec<LiveActivity>,
    pub rooms: Vec<RoomRow>,
    pub scenes: Vec<SceneCell>,
    pub room_ids: Vec<Id>,
    pub scene_ids: Vec<Id>,
}
pub fn path(file: &str) -> PathBuf {
    if let Some(root) = std::env::var_os("COUCH_HOME_DIR") {
        return PathBuf::from(root).join(file);
    }
    let root = if std::path::Path::new("/mnt/alpine/opt/couch").is_dir() {
        "/mnt/alpine/opt/couch"
    } else {
        "/opt/couch"
    };
    PathBuf::from(root).join(file)
}
pub fn read(previous: &str) -> Option<(String, Vec<Area>, [u8; 3])> {
    let raw = std::fs::read_to_string(path("config.json")).ok()?;
    if raw == previous {
        return None;
    }
    let config: Config = serde_json::from_str(&raw).ok()?;
    if config.schema_version != couch_model::SCHEMA_VERSION {
        return None;
    }
    Some((raw, project(&config), config.appearance.rgb()?))
}
fn project(config: &Config) -> Vec<Area> {
    let make = |name: String, ids: Vec<Id>, scene_ids: Vec<Id>| {
        let rooms: Vec<_> = ids.iter().filter_map(|id| config.room(id)).collect();
        Area {
            name,
            activities: Vec::new(),
            scenes: scene_ids
                .iter()
                .filter_map(|id| config.scene(id))
                .map(|s| SceneCell {
                    name: s.name.as_str().into(),
                    active: false,
                })
                .collect(),
            scene_ids,
            room_ids: rooms.iter().map(|r| r.id.clone()).collect(),
            rooms: rooms
                .iter()
                .map(|r| RoomRow {
                    name: r.name.as_str().into(),
                    devices: r.device_summary().into(),
                    detail: r.device_detail().into(),
                    active_count: 0,
                    status_known: false,
                    idle: true,
                    offline: false,
                    dimmed: false,
                    glyph: match r.effective_icon() {
                        couch_model::Icon::Sofa => 0,
                        couch_model::Icon::Bed => 1,
                        couch_model::Icon::CookingPot => 2,
                        couch_model::Icon::BookOpen => 3,
                        couch_model::Icon::DoorOpen => 4,
                        couch_model::Icon::Car => 5,
                        couch_model::Icon::Trees => 6,
                        couch_model::Icon::Lamp => 7,
                        couch_model::Icon::Tv => 8,
                        _ => 9,
                    },
                })
                .collect(),
        }
    };
    // All rooms remain reachable while the user is arranging their screens.
    let mut areas: Vec<_> = config
        .areas
        .iter()
        .map(|a| make(a.name.clone(), a.rooms.clone(), a.scenes.clone()))
        .collect();
    areas.push(make(
        "ALL ROOMS".into(),
        config.rooms.iter().map(|r| r.id.clone()).collect(),
        config.scenes.iter().map(|s| s.id.clone()).collect(),
    ));

    areas
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_room_is_reachable_without_configured_areas() {
        let mut config = Config::seed();
        config.areas.clear();
        let areas = project(&config);
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0].room_ids.len(), config.rooms.len());
        assert_eq!(
            areas[0].room_ids,
            config
                .rooms
                .iter()
                .map(|r| r.id.clone())
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn configured_screens_keep_their_room_order() {
        let mut config = Config::seed();
        config.areas[0].rooms.reverse();
        let areas = project(&config);
        assert_eq!(areas[0].name, config.areas[0].name);
        assert_eq!(areas[0].room_ids, config.areas[0].rooms);
        assert_eq!(areas.last().unwrap().room_ids.len(), config.rooms.len());
    }
}

/// Set both focus/text accent and its recessed tint from the saved RGB value.
pub fn apply_accent(app: &crate::App, rgb: [u8; 3]) {
    app.set_accent(slint::Color::from_rgb_u8(rgb[0], rgb[1], rgb[2]));
    let bg = [21u16, 19, 15];
    let tint = std::array::from_fn::<_, 3, _>(|i| ((rgb[i] as u16 * 15 + bg[i] * 85) / 100) as u8);
    app.set_accent_background(slint::Color::from_rgb_u8(tint[0], tint[1], tint[2]));
}
