use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong between here and the media box.
///
/// The endpoint is carried in most variants on purpose. This client ends up
/// inside a daemon whose log is the only diagnostic anyone has on the remote,
/// and "connection refused" without an address is a wasted line.
#[derive(Debug)]
pub enum Error {
    /// Could not reach the box, or the exchange broke part way through.
    Io { endpoint: String, source: io::Error },
    /// HTTP 401: the box has a web password set and we sent nothing, or the
    /// wrong thing.
    Unauthorized { endpoint: String },
    /// Any other non-200 status.
    Http {
        endpoint: String,
        status: u16,
        reason: String,
    },
    /// Something answered but it was not JSON-RPC. Usually a different service
    /// on that port, or Kodi with the web server on but remote control off.
    Protocol { endpoint: String, detail: String },
    /// The reply was JSON, but not the shape we asked to read it as.
    Json(serde_json::Error),
    /// Kodi understood the request and refused it.
    Rpc {
        method: String,
        code: i64,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { endpoint, source } => {
                // A socket timeout surfaces as WouldBlock on the BSDs and
                // TimedOut elsewhere. Neither "resource temporarily
                // unavailable" nor the bare kind tells a reader that the box
                // simply did not answer, which is the common case: it slept,
                // or it moved address.
                if matches!(
                    source.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) {
                    write!(f, "no answer from {endpoint} within the timeout")
                } else if source.kind() == io::ErrorKind::UnexpectedEof {
                    // We got as far as a connection, so "cannot reach" would
                    // send someone hunting the wrong fault.
                    write!(f, "{endpoint} dropped the connection")
                } else {
                    write!(f, "cannot reach {endpoint}: {source}")
                }
            }
            Error::Unauthorized { endpoint } => {
                write!(f, "{endpoint} wants a username and password")
            }
            Error::Http {
                endpoint,
                status,
                reason,
            } => {
                write!(f, "{endpoint} answered HTTP {status} {reason}")
            }
            Error::Protocol { endpoint, detail } => {
                write!(f, "{endpoint} is not answering JSON-RPC: {detail}")
            }
            Error::Json(e) => write!(f, "could not read Kodi's reply: {e}"),
            Error::Rpc {
                method,
                code,
                message,
            } => {
                write!(f, "Kodi refused {method}: {message} (code {code})")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}
