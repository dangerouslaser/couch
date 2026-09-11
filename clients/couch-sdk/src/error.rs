//! One error vocabulary, chosen to match what the broker already reports.
//!
//! `couch_control::Error` has five variants and the daemon and GUI both
//! render them to the user. A client that invents its own set has to be
//! translated twice, so this enum is those five plus the two mistakes a new
//! client makes most often - asking for a function it never declared, and
//! being handed settings that cannot address a device.

use std::fmt;

/// Why a request did not produce a result.
///
/// The mapping into the broker's type is total and lossless enough to be worth
/// writing down, because the wiring step in `couch-control` needs it:
///
/// | `couch_sdk::Error` | `couch_control::Error` |
/// |---|---|
/// | [`Error::Protocol`] | `Protocol` |
/// | [`Error::Transport`] | `Transport` |
/// | [`Error::Timeout`] | `Timeout` |
/// | [`Error::Rejected`] | `Rejected` |
/// | [`Error::Unsupported`] | `Remote(_)` |
/// | [`Error::Invalid`] | `Remote(_)` |
/// | [`Error::Remote`] | `Remote(_)` |
///
/// The broker's type lives in `clients/couch-control/src/lib.rs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The device answered, and the answer did not parse or did not confirm
    /// the request. Never retried: a device that replies with nonsense to one
    /// command replies with nonsense to the next one too.
    Protocol,
    /// The device could not be reached, or the connection dropped mid-request.
    Transport,
    /// No reply before the deadline. This does **not** prove the command was
    /// not executed, which is why nothing in this repository retries it.
    Timeout,
    /// The device understood the request and refused it - a revoked pairing, a
    /// locked input, an unsupported app.
    Rejected,
    /// The client does not implement this function. Returned by
    /// [`DeviceClient::command`](crate::DeviceClient::command) before any I/O
    /// when the requested function is absent from
    /// [`DeviceClient::capabilities`](crate::DeviceClient::capabilities).
    Unsupported,
    /// The settings cannot address a device: empty host, port 0, a URL with a
    /// control character in it. Raised before connecting.
    Invalid,
    /// Anything with a message worth showing. The daemon puts this string in a
    /// 502 body and the GUI puts it on screen, so write it for the person
    /// holding the remote, not for a log reader.
    Remote(String),
}

impl Error {
    /// A short message safe to show a user. Never include a credential here:
    /// these strings reach the browser UI and the device screen.
    pub fn message(&self) -> &str {
        match self {
            Self::Protocol => "The device sent a response this client could not use",
            Self::Transport => "The device could not be reached",
            Self::Timeout => "The device did not reply before the deadline",
            Self::Rejected => "The device refused the request",
            Self::Unsupported => "This device does not support that function",
            Self::Invalid => "These connection settings are incomplete",
            Self::Remote(message) => message,
        }
    }

    /// Whether a caller may reasonably try the *same* request again later.
    ///
    /// False for [`Error::Timeout`] on purpose. A lost reply does not prove a
    /// lost command, and repeating a power or input command that did in fact
    /// arrive is worse than reporting the failure.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Transport)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        match e.kind() {
            TimedOut | WouldBlock => Self::Timeout,
            PermissionDenied => Self::Remote(format!("Permission denied: {e}")),
            _ => Self::Transport,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::Protocol
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_kinds_map_to_the_variant_the_user_sees_and_timeouts_are_not_retryable() {
        let timeout: Error = std::io::Error::from(std::io::ErrorKind::TimedOut).into();
        assert_eq!(timeout, Error::Timeout);
        assert!(!timeout.retryable());
        let refused: Error = std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into();
        assert_eq!(refused, Error::Transport);
        assert!(refused.retryable());
        assert!(matches!(
            Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            Error::Remote(_)
        ));
        let bad: Error = serde_json::from_str::<u32>("{").unwrap_err().into();
        assert_eq!(bad, Error::Protocol);
        assert_eq!(
            Error::Remote("Pair this TV first".into()).to_string(),
            "Pair this TV first"
        );
    }
}
