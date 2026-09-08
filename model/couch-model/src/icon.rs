//! Stable Lucide names in configuration; compact catalog handles at runtime.
use alloc::string::String;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Icon(u16);
include!("icon_catalog.rs");
impl Default for Icon {
    fn default() -> Self {
        Self::House
    }
}
impl Icon {
    pub fn name(self) -> &'static str {
        ICON_NAMES[self.0 as usize]
    }
    pub fn from_name(name: &str) -> Option<Self> {
        ICON_NAMES.binary_search(&name).ok().map(|i| Self(i as u16))
    }
    /// Only for indexing the atlas generated alongside this catalog. Never persist it.
    pub fn catalog_index(self) -> usize {
        self.0 as usize
    }
    pub fn glyph_index(self) -> i32 {
        match self {
            Self::Sofa => 0,
            Self::Bed => 1,
            Self::CookingPot => 2,
            Self::BookOpen => 3,
            Self::DoorOpen => 4,
            Self::Car => 5,
            Self::Trees => 6,
            Self::Lamp => 7,
            Self::Tv => 8,
            _ => 9,
        }
    }
}
impl Serialize for Icon {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}
// Older/newer configurations can still load if an icon is unavailable.
impl<'de> Deserialize<'de> for Icon {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_name(&String::deserialize(d)?).unwrap_or_default())
    }
}
impl core::fmt::Display for Icon {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_catalog_icon_round_trips_by_name() {
        assert!(ALL_ICONS.len() > 2000);
        for icon in ALL_ICONS {
            assert_eq!(Icon::from_name(icon.name()), Some(*icon));
            let json = serde_json::to_string(icon).unwrap();
            assert_eq!(serde_json::from_str::<Icon>(&json).unwrap(), *icon);
        }
        assert_eq!(
            serde_json::from_str::<Icon>("\"unknown-future-icon\"").unwrap(),
            Icon::House
        );
        assert!(Icon::from_name("../../secret").is_none());
    }
}
