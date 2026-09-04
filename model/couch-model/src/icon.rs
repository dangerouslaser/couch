//! Named icons, rather than the GUI's `glyph: int`.
//!
//! `couch-gui` indexes into a list of embedded PNGs, which is the right thing
//! for it - the renderer has no font or SVG stack and the assets are baked in
//! at build time - but an index is a terrible thing to store. Inserting one
//! icon into `tools/mkuiicons.sh` would renumber every room in the file. The
//! config holds the Lucide name and [`Icon::glyph_index`] does the translation
//! at the boundary.

use alloc::string::String;

use serde::{Deserialize, Deserializer, Serialize};

/// The icons `tools/mkuiicons.sh` rasterises into `ui/couch-gui/assets`.
///
/// Keep this in step with that script's `ICONS` list; anything named here but
/// absent there renders as a house on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Icon {
    Sofa,
    Bed,
    CookingPot,
    BookOpen,
    DoorOpen,
    Car,
    Trees,
    Lamp,
    Tv,
    #[default]
    House,
    Lightbulb,
    Blinds,
    Thermometer,
    Speaker,
    Monitor,
}

/// Every icon, in the order the web UI should offer them.
pub const ALL_ICONS: &[Icon] = &[
    Icon::House,
    Icon::Sofa,
    Icon::Bed,
    Icon::CookingPot,
    Icon::BookOpen,
    Icon::DoorOpen,
    Icon::Car,
    Icon::Trees,
    Icon::Lamp,
    Icon::Tv,
    Icon::Lightbulb,
    Icon::Blinds,
    Icon::Thermometer,
    Icon::Speaker,
    Icon::Monitor,
];

impl Icon {
    pub fn name(self) -> &'static str {
        match self {
            Icon::Sofa => "sofa",
            Icon::Bed => "bed",
            Icon::CookingPot => "cooking-pot",
            Icon::BookOpen => "book-open",
            Icon::DoorOpen => "door-open",
            Icon::Car => "car",
            Icon::Trees => "trees",
            Icon::Lamp => "lamp",
            Icon::Tv => "tv",
            Icon::House => "house",
            Icon::Lightbulb => "lightbulb",
            Icon::Blinds => "blinds",
            Icon::Thermometer => "thermometer",
            Icon::Speaker => "speaker",
            Icon::Monitor => "monitor",
        }
    }

    pub fn from_name(name: &str) -> Option<Icon> {
        ALL_ICONS.iter().copied().find(|i| i.name() == name)
    }

    /// The index `RoomIcon` in `ui/screens/home_hub.slint` switches on.
    ///
    /// That component only draws the first ten; it falls through to the house
    /// for anything at or past nine, so the five icons the GUI has no case for
    /// map there deliberately rather than by accident.
    pub fn glyph_index(self) -> i32 {
        match self {
            Icon::Sofa => 0,
            Icon::Bed => 1,
            Icon::CookingPot => 2,
            Icon::BookOpen => 3,
            Icon::DoorOpen => 4,
            Icon::Car => 5,
            Icon::Trees => 6,
            Icon::Lamp => 7,
            Icon::Tv => 8,
            _ => 9,
        }
    }
}

/// Unknown names become the default rather than a parse failure.
///
/// The alternative is that a config written by a newer build - one that added
/// an icon - fails to load wholesale on an older device, losing the entire
/// house over a picture. `schema_version` is for changes that actually cannot
/// be read; this is not one of them.
impl<'de> Deserialize<'de> for Icon {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Icon, D::Error> {
        // Through `String` rather than `&str`: a borrowed deserialize only
        // works when the JSON is already in memory as text, and the daemon
        // reads its config through a reader.
        let name = String::deserialize(d)?;
        Ok(Icon::from_name(&name).unwrap_or_default())
    }
}

impl core::fmt::Display for Icon {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}
