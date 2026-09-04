use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong between the microphone and Home Assistant.
///
/// Two halves that never mix. The `Alsa`/`Audio` variants name a device node,
/// because on a board with twenty capture devices "invalid argument" without
/// one is a wasted line; the `Ws`/`Ha` variants name an endpoint, for the same
/// reason the Kodi client does.
///
/// There is deliberately no variant carrying a token. See `Ha::Unauthorized`:
/// it says the credential was refused and nothing about what it was.
#[derive(Debug)]
pub enum Error {
    /// The device node could not be opened, or an ioctl on it failed. `call`
    /// is the ALSA operation, so a reader can tell "cannot open" from "this
    /// card will not do 16 kHz".
    Alsa {
        device: String,
        call: &'static str,
        source: io::Error,
    },
    /// The hardware answered, but not with anything usable - no format in
    /// common, a capture that never produced a frame.
    Audio { device: String, detail: String },
    /// A WAV file that is not one, or not one we can use.
    Wav { path: String, detail: String },
    /// Could not reach Home Assistant, or the socket broke mid-exchange.
    Io { endpoint: String, source: io::Error },
    /// The WebSocket upgrade or framing went wrong. Usually something else
    /// listening on 8123, or a reverse proxy that does not pass upgrades.
    Ws { endpoint: String, detail: String },
    /// The long-lived access token was refused, or has expired.
    Unauthorized { endpoint: String },
    /// Home Assistant understood the request and refused it, or the pipeline
    /// reported an error event.
    Ha {
        endpoint: String,
        code: String,
        message: String,
    },
    /// The reply was JSON, but not the shape we asked to read it as.
    Json(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Alsa {
                device,
                call,
                source,
            } => {
                // ENOENT on a /dev/snd path is almost always "there is no such
                // capture device", not a missing driver, and saying so saves
                // someone checking whether ALSA is built in.
                if source.kind() == io::ErrorKind::NotFound {
                    write!(f, "{device} does not exist")
                } else if source.raw_os_error() == Some(libc::EBUSY) {
                    // EBUSY is the obvious "someone else has it", but on this
                    // MediaTek driver an unrouted analogue path gives the same
                    // errno with nothing holding the device at all - which
                    // reads as a mystery until you try `route`.
                    write!(
                        f,
                        "{device} is busy - either something else is recording, \
                         or the analogue path is not powered (try `couch-mic route`)"
                    )
                } else {
                    write!(f, "{device}: {call} failed: {source}")
                }
            }
            Error::Audio { device, detail } => write!(f, "{device}: {detail}"),
            Error::Wav { path, detail } => write!(f, "{path}: {detail}"),
            Error::Io { endpoint, source } => {
                // A socket timeout surfaces as WouldBlock on the BSDs and
                // TimedOut elsewhere, and neither bare kind tells a reader that
                // Home Assistant simply did not answer.
                if matches!(
                    source.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) {
                    write!(f, "no answer from {endpoint} within the timeout")
                } else if source.kind() == io::ErrorKind::UnexpectedEof {
                    write!(f, "{endpoint} dropped the connection")
                } else {
                    write!(f, "cannot reach {endpoint}: {source}")
                }
            }
            Error::Ws { endpoint, detail } => {
                write!(f, "{endpoint} is not speaking WebSocket: {detail}")
            }
            Error::Unauthorized { endpoint } => {
                write!(f, "{endpoint} refused the access token")
            }
            Error::Ha {
                endpoint,
                code,
                message,
            } => write!(f, "{endpoint} refused the pipeline: {message} ({code})"),
            Error::Json(e) => write!(f, "could not read Home Assistant's reply: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Alsa { source, .. } | Error::Io { source, .. } => Some(source),
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
