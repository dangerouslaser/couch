//! `couch-sonos` as a `couch-sdk` client.
//!
//! Nothing about the Control API transport changed: [`Client`] still reads the
//! household group list before a playback write, still refuses to forward a
//! command to another speaker, and still never retries. What this adds is the
//! part that was never Sonos-specific - the private per-connection settings
//! file written by the shared helper, and the function vocabulary declared once
//! where a test can compare it with the catalog the button picker reads.
//!
//! The dispatch below reuses the closed vocabulary [`Client::command`] already
//! accepts rather than restating it as a second table: the ids in
//! `capabilities()` are exactly the strings that method parses, and
//! `catalog_matches_the_model` is what keeps both equal to `couch-model`.

use std::net::Ipv4Addr;

use couch_sdk::{
    couch_model::commands::Function, Capability, ClientSettings, DeviceClient, Status,
};
use serde::{Deserialize, Serialize};

use crate::{Client, Error};

impl From<Error> for couch_sdk::Error {
    fn from(e: Error) -> Self {
        match &e {
            Error::Transport => Self::Transport,
            // It answered and the answer did not parse or did not confirm.
            Error::Response | Error::Http(_) => Self::Protocol,
            // The one variant that means the same thing on both sides.
            Error::Command => Self::Unsupported,
            // Everything else carries a message worth showing the person
            // holding the remote, and none of them can contain the API key.
            _ => Self::Remote(e.to_string()),
        }
    }
}

/// What Couch needs to reach one Sonos player.
///
/// The key is optional because it is a household credential rather than a
/// per-player one: a deployment may keep a single `sonos-api-key` file beside
/// `config.json` instead of repeating the same value in every connection. When
/// it is set here it is written to the per-connection file at mode 0600 like
/// any other credential.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// The player's IPv4 address. Sonos serves the API on port 1443, so there
    /// is no port to configure.
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl Settings {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            api_key: None,
        }
    }
    /// Environment first, then this connection's own settings, then the
    /// household file, then the placeholder. Never logged.
    pub fn key(&self) -> String {
        crate::choose_key([
            crate::key_from_env(),
            self.api_key.clone(),
            std::fs::read_to_string(crate::key_file()).ok(),
        ])
    }
    fn address(&self) -> couch_sdk::Result<Ipv4Addr> {
        self.host
            .trim()
            .parse()
            .map_err(|_| couch_sdk::Error::Invalid)
    }
}

impl ClientSettings for Settings {
    const FILE_PREFIX: &'static str = "sonos";

    fn validate(&self) -> couch_sdk::Result<()> {
        self.address()?;
        // A key that cannot be a header value is rejected here rather than at
        // the point of use, where it would become a request that never went out.
        if self
            .api_key
            .as_deref()
            .is_some_and(|key| !crate::key_ok(key.trim()))
        {
            return Err(couch_sdk::Error::Invalid);
        }
        Ok(())
    }
}

impl DeviceClient for Client {
    type Settings = Settings;

    const KIND: &'static str = "sonos";
    const LABEL: &'static str = "Sonos";

    /// Exactly `couch_model::buttons::functions(&Integration::Sonos { .. })`,
    /// and `catalog_matches_the_model` fails if that stops being true.
    ///
    /// No power keys: a Sonos player has no power state to set or observe, and
    /// offering one would put a key on screen that silently does nothing.
    fn capabilities() -> &'static [Capability] {
        &[
            ("play", "Play"),
            ("pause", "Pause"),
            ("play-pause", "Play / pause"),
            ("stop", "Stop"),
            ("next", "Next"),
            ("previous", "Previous"),
            ("volume-up", "Volume up"),
            ("volume-down", "Volume down"),
            ("mute", "Mute"),
            ("mute-on", "Mute on"),
            ("mute-off", "Mute off"),
        ]
    }

    fn connect(settings: &Settings) -> couch_sdk::Result<Self> {
        settings.validate()?;
        Client::connect_with_key(settings.address()?, &settings.key()).map_err(Into::into)
    }

    /// One dispatch, not two: the declared ids are the vocabulary
    /// [`Client::command`] parses, including the group check before a playback
    /// write and the read-then-write pair behind a mute toggle.
    fn execute(&mut self, function: &Function) -> couch_sdk::Result<()> {
        let id = function.id();
        if !Self::capabilities().iter().any(|(name, _)| *name == id) {
            return Err(couch_sdk::Error::Unsupported);
        }
        Client::command(self, &id).map_err(Into::into)
    }

    /// `on` stays absent: a player reports no power state, and inventing one
    /// from "it answered" would be a different fact. Playback is the group's;
    /// volume and mute are this player's.
    fn status(&mut self) -> couch_sdk::Result<Status> {
        let state = Client::status(self)?;
        let status = Status::default()
            .with_muted(state.muted)
            .with_volume(state.volume)?;
        Ok(match state.transport.as_str() {
            "PLAYING" | "BUFFERING" => status.with_playing(true),
            "PAUSED" | "IDLE" => status.with_playing(false),
            // An unfamiliar transport state is not an assertion about playback.
            _ => status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{groups, info, server, volume, KEY, PLAYER};
    use couch_sdk::couch_model::Integration;
    use couch_sdk::testing::{contract_findings, MockHost, Reply, Script};

    fn integration() -> Integration {
        Integration::Sonos {
            host: "192.0.2.10".into(),
        }
    }

    #[test]
    fn catalog_matches_the_model_so_the_button_picker_offers_what_this_client_implements() {
        assert_eq!(
            couch_sdk::catalog_differences::<Client>(&integration()),
            Vec::<String>::new()
        );
        // And every declared function is one this registered provider accepts.
        for (id, _) in <Client as DeviceClient>::capabilities() {
            let function = Function::parse(id).unwrap();
            assert!(function.supports(&integration()), "{id}");
        }
    }

    /// The SDK's mock host is a scripted line protocol; this client speaks
    /// HTTPS and JSON to a fixed port, so no settings can point it at one.
    /// Everything the checker decides before connecting still runs here - the
    /// slug, the label, every declared function being canonical, unique and
    /// accepted by the gate, and the settings surviving a real 0600 file - and
    /// `connect failed` is the only finding left, exactly as in `couch-echo`'s
    /// unreachable-host test. The gate itself is proved below against an HTTP
    /// fixture the client can actually speak to.
    #[test]
    fn the_contract_holds_except_where_the_mock_host_cannot_speak_https() {
        let host = MockHost::start(Script::new().otherwise(Reply::Close));
        let settings = Settings {
            // Loopback refuses port 1443 immediately rather than waiting out
            // the five-second deadline.
            host: host.host().into(),
            api_key: Some("fixture-key".into()),
        };
        let findings = contract_findings::<Client>(&settings, &host);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].starts_with("connect failed"), "{findings:?}");
        assert!(
            host.requests().is_empty(),
            "nothing reached the observer: {:?}",
            host.requests()
        );
    }

    #[test]
    fn an_undeclared_function_is_refused_before_any_request() {
        let (base, thread) = server(vec![(200, info())]);
        let mut client = Client::connect_url(&base, KEY).unwrap();
        for refused in [
            "power-off",
            "home",
            "wash-the-dishes",
            "volume",
            "app:spotify",
            "input:hdmi1",
        ] {
            assert_eq!(
                DeviceClient::command(&mut client, refused),
                Err(couch_sdk::Error::Unsupported),
                "{refused} is not declared and must be refused before any I/O"
            );
        }
        assert_eq!(
            thread.join().unwrap().len(),
            1,
            "only connect's identifying request: a refusal costs no round trip"
        );
    }

    #[test]
    fn a_refused_playback_command_is_reported_once_and_never_retried() {
        let refusal = r#"{"_objectType":"playbackError","errorCode":"ERROR_PLAYBACK_NO_CONTENT"}"#;
        let (base, thread) = server(vec![
            (200, info()),
            (200, groups(PLAYER, "PLAYBACK_STATE_IDLE")),
            (499, refusal.into()),
        ]);
        let mut client = Client::connect_url(&base, KEY).unwrap();
        assert_eq!(
            DeviceClient::command(&mut client, "play"),
            Err(couch_sdk::Error::Remote(
                "Sonos API error ERROR_PLAYBACK_NO_CONTENT".into()
            )),
            "the player's own code reaches the person holding the remote"
        );
        assert_eq!(
            thread.join().unwrap().len(),
            3,
            "one topology read and one write, never a second attempt"
        );
    }

    #[test]
    fn a_reading_reports_what_the_group_and_the_player_said() {
        for (state, playing) in [
            ("PLAYBACK_STATE_PLAYING", Some(true)),
            ("PLAYBACK_STATE_BUFFERING", Some(true)),
            ("PLAYBACK_STATE_PAUSED", Some(false)),
            ("PLAYBACK_STATE_IDLE", Some(false)),
            ("PLAYBACK_STATE_INVENTED_BY_NEWER_FIRMWARE", None),
        ] {
            let (base, thread) = server(vec![
                (200, info()),
                (200, groups(PLAYER, state)),
                (200, volume(17, true)),
            ]);
            let mut client = Client::connect_url(&base, KEY).unwrap();
            let status = DeviceClient::status(&mut client).unwrap();
            assert_eq!(status.playing, playing, "{state}");
            assert_eq!(status.muted, Some(true));
            assert_eq!(status.volume, Some(17));
            assert_eq!(status.on, None, "a player has no power state to report");
            assert_eq!(status.input, None);
            thread.join().unwrap();
        }
    }

    #[test]
    fn settings_reject_what_cannot_address_a_player_or_sign_a_request() {
        assert!(Settings::new(" 192.0.2.10 ").validate().is_ok());
        for bad in [
            "",
            "sonos.local",
            "192.0.2.10:1443",
            "192.0.2",
            "2001:db8::1",
            "192.0.2.10 evil",
        ] {
            assert_eq!(
                Settings::new(bad).validate(),
                Err(couch_sdk::Error::Invalid),
                "{bad}"
            );
        }
        let mut settings = Settings::new("192.0.2.10");
        settings.api_key = Some("key\nX-Sonos-Api-Key: other".into());
        assert_eq!(settings.validate(), Err(couch_sdk::Error::Invalid));
    }

    #[test]
    fn a_connection_key_is_preferred_to_the_household_file() {
        if std::env::var_os(crate::KEY_ENV).is_some() {
            return; // An operator override outranks both and would mask this.
        }
        let mut settings = Settings::new("192.0.2.10");
        assert_eq!(
            settings.key(),
            crate::api_key(),
            "without one of its own, a connection uses the household key"
        );
        settings.api_key = Some("per-connection".into());
        assert_eq!(settings.key(), "per-connection");
    }
}
