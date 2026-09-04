//! Dialling, shared by both transports.

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// Bracket IPv6 literals, which is the only reason this is not a format!.
pub(crate) fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Connect within `timeout`, whatever the host resolves to.
///
/// Name resolution is not covered by connect_timeout. The daemon is pointed at
/// a LAN address or an mDNS name, both of which answer or fail promptly; a name
/// that needs a dead DNS server is the one case that can outlast the timeout,
/// and there is no fixing that without a resolver of our own.
///
/// The deadline covers all candidate addresses rather than each one, so a host
/// with an unreachable AAAA and a working A cannot cost two timeouts. The
/// caller was promised one.
pub(crate) fn connect(
    host: &str,
    port: u16,
    timeout: Duration,
    endpoint: &str,
) -> Result<TcpStream> {
    let io_err = |e: io::Error| Error::Io {
        endpoint: endpoint.to_string(),
        source: e,
    };

    let addrs = (host, port).to_socket_addrs().map_err(io_err)?;
    let deadline = Instant::now() + timeout;
    let mut last: Option<io::Error> = None;
    for addr in addrs {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&addr, left) {
            Ok(s) => {
                s.set_read_timeout(Some(timeout)).map_err(io_err)?;
                s.set_write_timeout(Some(timeout)).map_err(io_err)?;
                return Ok(s);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(io_err(last.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "ran out of time trying every address",
        )
    })))
}

/// A socket timeout surfaces as WouldBlock on the BSDs and TimedOut elsewhere.
pub(crate) fn timed_out(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}
