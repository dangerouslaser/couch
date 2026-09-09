//! Per-connection private credentials and broker proxy for streaming TVs.
use super::*;
use std::os::unix::fs::OpenOptionsExt;
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StreamingConnection {
    AndroidTv {
        settings: couch_androidtv::Settings,
        credentials: couch_androidtv::Credentials,
    },
    AppleTv {
        settings: couch_appletv::Settings,
        credentials: couch_appletv::Credentials,
    },
}
impl std::fmt::Debug for StreamingConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StreamingConnection([redacted])")
    }
}
impl StreamingConnection {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AndroidTv { .. } => "androidtv",
            Self::AppleTv { .. } => "appletv",
        }
    }
    pub fn address(&self) -> std::net::IpAddr {
        match self {
            Self::AndroidTv { settings, .. } => settings.address,
            Self::AppleTv { settings, .. } => settings.address,
        }
    }
    pub fn port(&self) -> u16 {
        match self {
            Self::AndroidTv { settings, .. } => settings.remote_port,
            Self::AppleTv { settings, .. } => settings.companion_port,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let address = self.address();
        if address.is_unspecified() || address.is_multicast() || self.port() == 0 {
            return Err(Error::Protocol);
        }
        match self {
            Self::AndroidTv {
                settings,
                credentials,
            } => {
                if settings.pairing_port == 0
                    || credentials.server_certificate_der.is_empty()
                    || credentials.server_certificate_der.len() > 8192
                    || credentials.identity.certificate_der.is_empty()
                    || credentials.identity.certificate_der.len() > 8192
                    || credentials.identity.private_key_pkcs8.is_empty()
                    || credentials.identity.private_key_pkcs8.len() > 8192
                {
                    return Err(Error::Protocol);
                }
            }
            Self::AppleTv { credentials, .. } => {
                if credentials.client_id.is_empty()
                    || credentials.client_id.len() > 128
                    || credentials.device_id.is_empty()
                    || credentials.device_id.len() > 128
                {
                    return Err(Error::Protocol);
                }
            }
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut bytes = vec![];
        std::io::Read::by_ref(&mut file)
            .take(65537)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            return Err(Error::Protocol);
        }
        let value: Self = serde_json::from_slice(&bytes)?;
        value.validate()?;
        Ok(value)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let temporary = path.with_extension(format!("{}.new", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(&serde_json::to_vec(self)?)?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)?;
            if let Some(parent) = path.parent() {
                std::fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}
pub struct StreamingTv {
    handle: Handle,
}
impl StreamingTv {
    pub fn connect(settings: &StreamingConnection) -> Result<Self> {
        settings.validate()?;
        let c = Self {
            handle: Handle::new(Spec::Streaming(settings.clone())),
        };
        c.handle.call(Op::Open)?;
        Ok(c)
    }
    pub fn matches(&self, settings: &StreamingConnection) -> bool {
        matches!(&self.handle.spec,Spec::Streaming(saved) if serde_json::to_vec(saved).ok()==serde_json::to_vec(settings).ok())
    }
    pub fn status(&self) -> Result<Value> {
        self.handle.call(Op::StreamingStatus)
    }
    pub fn apps(&self) -> Result<Value> {
        self.handle.call(Op::StreamingApps)
    }
    pub fn command(&self, function: &str) -> Result<()> {
        if function.len() > 2176 {
            return Err(Error::Protocol);
        }
        self.handle.get(Op::StreamingCommand(function.into()))
    }
}
pub(super) enum Client {
    Android(couch_androidtv::Remote),
    Apple(couch_appletv::Remote),
}
impl Client {
    pub fn open(s: &StreamingConnection) -> Result<Self> {
        s.validate()?;
        Ok(match s {
            StreamingConnection::AndroidTv {
                settings,
                credentials,
            } => Self::Android(
                couch_androidtv::Remote::connect(settings, credentials)
                    .map_err(|e| Error::Remote(e.to_string()))?,
            ),
            StreamingConnection::AppleTv {
                settings,
                credentials,
            } => Self::Apple(
                couch_appletv::Remote::connect(settings, credentials)
                    .map_err(|e| Error::Remote(e.to_string()))?,
            ),
        })
    }
    pub fn idle(&mut self) -> Result<()> {
        match self {
            Self::Android(c) => {
                c.poll().map_err(|e| Error::Remote(e.to_string()))?;
            }
            Self::Apple(c) => {
                c.poll().map_err(|e| Error::Remote(e.to_string()))?;
            }
        }
        Ok(())
    }
    pub fn status(&mut self) -> Result<Value> {
        self.idle()?;
        Ok(match self {
            Self::Android(c) => {
                json!({"connected":true,"on":c.state.on,"volume":c.state.volume,"model":c.state.model,"vendor":c.state.vendor})
            }
            Self::Apple(_) => {
                json!({"connected":true,"protocol":"companion","now_playing_supported":false})
            }
        })
    }
    pub fn apps(&mut self) -> Result<Value> {
        match self {
            Self::Apple(c) => c
                .launchable_apps()
                .map(|apps| json!({"apps": apps}))
                .map_err(|error| Error::Remote(error.to_string())),
            Self::Android(_) => Err(Error::Remote(
                "Android TV does not provide installed app discovery".into(),
            )),
        }
    }
    pub fn command(&mut self, name: &str) -> Result<()> {
        match self {
            Self::Android(c) => {
                use couch_androidtv::Button as B;
                if let Some(url) = name.strip_prefix("app:") {
                    return c.launch(url).map_err(|e| Error::Remote(e.to_string()));
                }
                let b = match name {
                    "up" => B::Up,
                    "down" => B::Down,
                    "left" => B::Left,
                    "right" => B::Right,
                    "ok" => B::Ok,
                    "back" => B::Back,
                    "home" => B::Home,
                    "menu" => B::Menu,
                    "volume-up" => B::VolumeUp,
                    "volume-down" => B::VolumeDown,
                    "mute" => B::Mute,
                    "power-on" => B::PowerOn,
                    "power-off" => B::PowerOff,
                    "play" => B::Play,
                    "pause" => B::Pause,
                    "play-pause" => B::PlayPause,
                    "stop" => B::Stop,
                    "next" => B::Next,
                    "previous" => B::Previous,
                    "rewind" => B::Rewind,
                    "fast-forward" => B::FastForward,
                    "channel-up" => B::ChannelUp,
                    "channel-down" => B::ChannelDown,
                    _ => return Err(Error::Protocol),
                };
                c.press(b).map_err(|e| Error::Remote(e.to_string()))
            }
            Self::Apple(c) => {
                use couch_appletv::{Button as B, Playback as P};
                if let Some(id) = name.strip_prefix("app:") {
                    return c.launch(id).map_err(|e| Error::Remote(e.to_string()));
                }
                let playback = match name {
                    "play" => Some(P::Play),
                    "pause" => Some(P::Pause),
                    "next" => Some(P::Next),
                    "previous" => Some(P::Previous),
                    _ => None,
                };
                if let Some(playback) = playback {
                    return c
                        .playback(playback)
                        .map_err(|e| Error::Remote(e.to_string()));
                }
                let b = match name {
                    "up" => B::Up,
                    "down" => B::Down,
                    "left" => B::Left,
                    "right" => B::Right,
                    "ok" => B::Ok,
                    "back" => B::Back,
                    "home" => B::Home,
                    "volume-up" => B::VolumeUp,
                    "volume-down" => B::VolumeDown,
                    "power-on" => B::PowerOn,
                    "power-off" => B::PowerOff,
                    "play-pause" => B::PlayPause,
                    "channel-up" => B::ChannelUp,
                    "channel-down" => B::ChannelDown,
                    _ => return Err(Error::Protocol),
                };
                c.press(b).map_err(|e| Error::Remote(e.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_private_bounded_and_redacted() {
        let path =
            std::env::temp_dir().join(format!("couch-streaming-test-{}.json", std::process::id()));
        let settings = StreamingConnection::AppleTv {
            settings: couch_appletv::Settings {
                address: "127.0.0.1".parse().unwrap(),
                companion_port: 49152,
            },
            credentials: couch_appletv::Credentials {
                client_id: b"fixture-client".to_vec(),
                client_secret: [7; 32],
                device_id: b"fixture-tv".to_vec(),
                device_public: [8; 32],
            },
        };
        settings.save(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(StreamingConnection::load(&path).unwrap().kind(), "appletv");
        assert!(!format!("{settings:?}").contains("fixture-client"));
        std::fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(StreamingConnection::load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
