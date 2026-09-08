use alloc::string::String;
use serde::{Deserialize, Serialize};

/// Presentation only: colors never alter device or connection configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default = "default_accent")]
    pub accent: String,
}
fn default_accent() -> String {
    "#E8703A".into()
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            accent: default_accent(),
        }
    }
}
impl Appearance {
    pub fn rgb(&self) -> Option<[u8; 3]> {
        let hex = self.accent.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        ])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_color_roundtrips_and_invalid_colors_are_rejected() {
        let a = Appearance {
            accent: "#b794f4".into(),
        };
        assert_eq!(a.rgb(), Some([183, 148, 244]));
        assert_eq!(
            serde_json::from_str::<Appearance>(&serde_json::to_string(&a).unwrap()).unwrap(),
            a
        );
        for bad in ["purple", "#123", "#11223344", "#zzzzzz", "#🟣🟣"] {
            assert!(Appearance { accent: bad.into() }.rgb().is_none());
        }
    }
    #[test]
    fn old_config_keeps_its_accent() {
        let c: crate::Config = serde_json::from_str(r#"{"schema_version":1}"#).unwrap();
        assert_eq!(c.appearance.rgb(), Some([232, 112, 58]));
    }
}
