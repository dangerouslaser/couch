//! Typed executable functions. Strings exist only at persisted/UI boundaries;
//! execution matches this enum and capability checks use the same vocabulary.
use crate::Integration;
use alloc::{
    format,
    string::{String, ToString},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Function {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
    Home,
    Menu,
    VolumeUp,
    VolumeDown,
    Mute,
    MuteOn,
    MuteOff,
    PowerOn,
    PowerOff,
    ChannelUp,
    ChannelDown,
    Red,
    Green,
    Blue,
    Yellow,
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    Rewind,
    FastForward,
    On,
    Off,
    Toggle,
    Input(String),
    App(String),
}
impl Function {
    pub fn parse(value: &str) -> Option<Self> {
        if let Some(id) = value.strip_prefix("input:").filter(|s| valid_id(s)) {
            return Some(Self::Input(id.into()));
        }
        if let Some(id) = value.strip_prefix("app:").filter(|s| valid_id(s)) {
            return Some(Self::App(id.into()));
        }
        Some(match value {
            "up" => Self::Up,
            "down" => Self::Down,
            "left" => Self::Left,
            "right" => Self::Right,
            "ok" => Self::Ok,
            "back" => Self::Back,
            "home" => Self::Home,
            "menu" => Self::Menu,
            "volume-up" => Self::VolumeUp,
            "volume-down" => Self::VolumeDown,
            "mute" => Self::Mute,
            "mute-on" => Self::MuteOn,
            "mute-off" => Self::MuteOff,
            "power-on" => Self::PowerOn,
            "power-off" => Self::PowerOff,
            "channel-up" => Self::ChannelUp,
            "channel-down" => Self::ChannelDown,
            "red" => Self::Red,
            "green" => Self::Green,
            "blue" => Self::Blue,
            "yellow" => Self::Yellow,
            "play" => Self::Play,
            "pause" => Self::Pause,
            "play-pause" => Self::PlayPause,
            "stop" => Self::Stop,
            "next" => Self::Next,
            "previous" => Self::Previous,
            "rewind" => Self::Rewind,
            "fast-forward" => Self::FastForward,
            "on" => Self::On,
            "off" => Self::Off,
            "toggle" => Self::Toggle,
            _ => return None,
        })
    }
    pub fn id(&self) -> String {
        match self {
            Self::Input(id) => format!("input:{id}"),
            Self::App(id) => format!("app:{id}"),
            _ => match self {
                Self::Up => "up",
                Self::Down => "down",
                Self::Left => "left",
                Self::Right => "right",
                Self::Ok => "ok",
                Self::Back => "back",
                Self::Home => "home",
                Self::Menu => "menu",
                Self::VolumeUp => "volume-up",
                Self::VolumeDown => "volume-down",
                Self::Mute => "mute",
                Self::MuteOn => "mute-on",
                Self::MuteOff => "mute-off",
                Self::PowerOn => "power-on",
                Self::PowerOff => "power-off",
                Self::ChannelUp => "channel-up",
                Self::ChannelDown => "channel-down",
                Self::Red => "red",
                Self::Green => "green",
                Self::Blue => "blue",
                Self::Yellow => "yellow",
                Self::Play => "play",
                Self::Pause => "pause",
                Self::PlayPause => "play-pause",
                Self::Stop => "stop",
                Self::Next => "next",
                Self::Previous => "previous",
                Self::Rewind => "rewind",
                Self::FastForward => "fast-forward",
                Self::On => "on",
                Self::Off => "off",
                Self::Toggle => "toggle",
                _ => unreachable!(),
            }
            .to_string(),
        }
    }
    pub fn supports(&self, integration: &Integration) -> bool {
        match self {
            Self::Input(id) => match integration {
                Integration::WebOs => valid_id(id),
                Integration::Denon { .. } => {
                    id.len() <= 25
                        && id.bytes().all(|b| {
                            b.is_ascii_uppercase() || b.is_ascii_digit() || b" /+-".contains(&b)
                        })
                }
                _ => false,
            },
            Self::App(id) => matches!(integration, Integration::WebOs | Integration::AppleTv) && valid_id(id),
            _ => crate::buttons::functions(integration)
                .iter()
                .any(|f| f.0 == self.id()),
        }
    }
    pub fn repeatable(&self) -> bool {
        matches!(
            self,
            Self::Up
                | Self::Down
                | Self::Left
                | Self::Right
                | Self::VolumeUp
                | Self::VolumeDown
                | Self::ChannelUp
                | Self::ChannelDown
        )
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/-+".contains(&b))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_catalog_entries_round_trip_and_dynamic_functions_are_provider_checked() {
        for integration in [
            Integration::Kodi {
                host: "host".into(),
                port: 9090,
            },
            Integration::WebOs,
            Integration::Denon {
                host: "host".into(),
                port: 23,
            },
            Integration::Hue {
                light_id: "id".into(),
            },
            Integration::HomeAssistant {
                entity_id: "light.test".into(),
            },
        ] {
            for (id, _) in crate::buttons::functions(&integration) {
                let f = Function::parse(id).unwrap();
                assert_eq!(f.id(), *id);
                assert!(f.supports(&integration));
            }
        }
        assert!(Function::parse("input:HDMI_1")
            .unwrap()
            .supports(&Integration::WebOs));
        assert!(!Function::parse("input:HDMI_1")
            .unwrap()
            .supports(&Integration::Denon {
                host: "h".into(),
                port: 23
            }));
        assert!(Function::parse("input:BD\rMV98").is_none());
        assert!(Function::parse("arbitrary-rpc").is_none());
    }
}
