use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong between a button name and light out the front.
///
/// The split is deliberate: [`Encode`] means the request was wrong (an
/// out-of-range address, an unknown protocol, a codeset line that does not
/// parse) and no hardware was touched; [`Irtx`] means the request was fine but
/// the blaster refused it, and it always names `/dev/irtx` and the call,
/// because on a device where every driver returns `EINVAL` the same way, "which
/// syscall" is the difference between a wrong ioctl number and a wrong buffer.
#[derive(Debug)]
pub enum Error {
    /// The command could not be turned into a frame. Free text because the
    /// causes are all "you asked for something that is not a thing": a Sony
    /// command that does not fit its bit width, a hex value that will not
    /// parse, a raw table with no timings.
    Encode(String),
    /// A codeset file that is not one, or a line in it that does not parse.
    /// `path` and `line` locate it; `detail` says what was wrong.
    Codeset {
        path: String,
        line: usize,
        detail: String,
    },
    /// The blaster could not be opened, or an ioctl/write on it failed. `call`
    /// is the operation so a reader can tell "cannot open" from "the driver
    /// rejected the carrier ioctl" from "the write was short".
    Irtx {
        device: String,
        call: &'static str,
        source: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Encode(m) => write!(f, "{m}"),
            Error::Codeset { path, line, detail } => {
                write!(f, "{path}:{line}: {detail}")
            }
            Error::Irtx {
                device,
                call,
                source,
            } => {
                // ENOENT on /dev/irtx is "this kernel has no blaster", not a
                // missing file the user should create - the driver is what
                // makes the node, so saying so points at the right thing.
                if source.kind() == io::ErrorKind::NotFound {
                    write!(
                        f,
                        "{device} does not exist - the mt_irtx driver is not \
                         loaded, or this is not the HA100"
                    )
                } else if source.raw_os_error() == Some(libc::EBUSY) {
                    // The driver is single-open: one writer at a time. This is
                    // the errno you get when couch-gui or a second couch-ir is
                    // already holding it.
                    write!(f, "{device} is busy - something else has the blaster open")
                } else if source.raw_os_error() == Some(libc::ENOTTY) {
                    // ENOTTY from an ioctl is a wrong ioctl number: the driver
                    // is working, it just does not know the request. That means
                    // the ABI in src/abi.rs is wrong for this kernel, which is
                    // exactly the thing docs/ir.md flags as needing the device.
                    write!(
                        f,
                        "{device}: {call} returned ENOTTY - the mt_irtx ioctl ABI \
                         does not match this kernel (see docs/ir.md)"
                    )
                } else {
                    write!(f, "{device}: {call} failed: {source}")
                }
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Irtx { source, .. } => Some(source),
            _ => None,
        }
    }
}
