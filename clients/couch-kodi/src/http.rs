//! Just enough HTTP/1.1 to post a JSON-RPC body and read the answer.
//!
//! Kodi is on the LAN and speaks plain HTTP, so there is no TLS to justify a
//! client stack. What is left - one POST, a status line, a handful of headers
//! and a body in one of three framings - is smaller than the glue any crate
//! would need, and it keeps the armv7 cross-compile down to serde.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::net;

/// A JSON-RPC reply about one playing item is a few kilobytes. Anything near
/// this is a different service or a bug, and buffering it on a 1GB device to
/// find that out is not worth it.
const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_LINE: u64 = 16 * 1024;

pub(crate) struct Http {
    pub host: String,
    pub port: u16,
    /// Pre-encoded "Basic <base64>", so the password is encoded once rather
    /// than on every call.
    pub auth: Option<String>,
    pub timeout: Duration,
}

impl Http {
    pub fn endpoint(&self) -> String {
        format!("http://{}/jsonrpc", self.authority())
    }

    pub fn authority(&self) -> String {
        net::authority(&self.host, self.port)
    }

    /// Post the request and hand back the reply object.
    pub fn exchange(&self, request: &Value, _id: u64) -> Result<Value> {
        let raw = self.post_json(&serde_json::to_vec(request)?)?;
        serde_json::from_slice(&raw).map_err(|e| Error::Protocol {
            endpoint: self.endpoint(),
            // The body matters here: this is the error you get from a web
            // server that is not Kodi, and its first line usually names it.
            detail: format!(
                "{e} (body began {:?})",
                String::from_utf8_lossy(&raw[..raw.len().min(72)])
            ),
        })
    }

    fn post_json(&self, body: &[u8]) -> Result<Vec<u8>> {
        let endpoint = self.endpoint();
        let io_err = |e: io::Error| Error::Io {
            endpoint: endpoint.clone(),
            source: e,
        };
        let bad = |detail: String| Error::Protocol {
            endpoint: endpoint.clone(),
            detail,
        };

        let stream = net::connect(&self.host, self.port, self.timeout, &endpoint)?;

        let mut head = String::with_capacity(256);
        head.push_str("POST /jsonrpc HTTP/1.1\r\n");
        let _ = write!(head, "Host: {}\r\n", self.authority());
        head.push_str("Content-Type: application/json\r\n");
        let _ = write!(head, "Content-Length: {}\r\n", body.len());
        // No keep-alive: one call per connection costs a handshake on a LAN and
        // saves having to reason about a pooled socket that the box dropped
        // while it was asleep.
        head.push_str("Connection: close\r\n");
        if let Some(auth) = &self.auth {
            let _ = write!(head, "Authorization: {auth}\r\n");
        }
        head.push_str("\r\n");

        let mut req = head.into_bytes();
        req.extend_from_slice(body);
        let mut w = &stream;
        w.write_all(&req).map_err(io_err)?;
        w.flush().map_err(io_err)?;

        let mut r = BufReader::new(&stream);

        let status_line = match read_line(&mut r).map_err(io_err)? {
            Some(l) => l,
            None => return Err(bad("it closed the connection without answering".into())),
        };
        if !status_line.starts_with("HTTP/") {
            return Err(bad(format!("expected a status line, got {status_line:?}")));
        }
        let mut parts = status_line.splitn(3, ' ');
        parts.next();
        let status: u16 = match parts.next().and_then(|s| s.parse().ok()) {
            Some(s) => s,
            None => return Err(bad(format!("unreadable status line {status_line:?}"))),
        };
        let reason = parts.next().unwrap_or("").trim().to_string();

        let mut length: Option<usize> = None;
        let mut chunked = false;
        loop {
            match read_line(&mut r).map_err(io_err)? {
                None => return Err(bad("it closed the connection mid-header".into())),
                Some(l) if l.is_empty() => break,
                Some(l) => {
                    if let Some((name, value)) = l.split_once(':') {
                        match name.trim().to_ascii_lowercase().as_str() {
                            "content-length" => length = value.trim().parse().ok(),
                            "transfer-encoding" => {
                                chunked = value.to_ascii_lowercase().contains("chunked")
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        match status {
            200 => {}
            401 => {
                return Err(Error::Unauthorized {
                    endpoint: endpoint.clone(),
                })
            }
            _ => {
                return Err(Error::Http {
                    endpoint: endpoint.clone(),
                    status,
                    reason,
                })
            }
        }

        let mut out = Vec::new();
        if chunked {
            // Kodi answers chunked often enough - it depends on the handler and
            // the build - that ignoring this framing gets you a body with hex
            // lengths sprinkled through it.
            read_chunked(&mut r, &mut out).map_err(|e| match e.kind() {
                io::ErrorKind::InvalidData => bad(e.to_string()),
                _ => io_err(e),
            })?;
        } else if let Some(n) = length {
            if n > MAX_BODY {
                return Err(bad(format!(
                    "{n} byte body is far larger than any RPC reply"
                )));
            }
            (&mut r)
                .take(n as u64)
                .read_to_end(&mut out)
                .map_err(io_err)?;
            if out.len() != n {
                return Err(bad(format!("body stopped at {} of {n} bytes", out.len())));
            }
        } else {
            // Neither framing header: the body runs to the close we asked for.
            (&mut r)
                .take(MAX_BODY as u64)
                .read_to_end(&mut out)
                .map_err(io_err)?;
        }
        Ok(out)
    }
}

/// One CRLF-terminated line, without the terminator. `None` at EOF.
fn read_line<R: BufRead>(r: &mut R) -> io::Result<Option<String>> {
    let mut raw = Vec::new();
    // Capped, because a server that never sends a newline would otherwise be
    // able to grow this until the device runs out of memory.
    if (&mut *r).take(MAX_LINE).read_until(b'\n', &mut raw)? == 0 {
        return Ok(None);
    }
    while matches!(raw.last(), Some(b'\n' | b'\r')) {
        raw.pop();
    }
    Ok(Some(String::from_utf8_lossy(&raw).into_owned()))
}

fn read_chunked<R: BufRead>(r: &mut R, out: &mut Vec<u8>) -> io::Result<()> {
    let bad = |m: String| io::Error::new(io::ErrorKind::InvalidData, m);
    loop {
        let line = read_line(r)?
            .ok_or_else(|| bad("the connection closed part way through a chunk".into()))?;
        let head = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(head, 16)
            .map_err(|_| bad(format!("{head:?} is not a chunk length")))?;
        if size == 0 {
            // Trailers may follow the final chunk. We asked for the connection
            // to close and never reuse it, so there is nothing to drain.
            return Ok(());
        }
        if out.len() + size > MAX_BODY {
            return Err(bad("chunked body is far larger than any RPC reply".into()));
        }
        let at = out.len();
        out.resize(at + size, 0);
        r.read_exact(&mut out[at..])?;
        let _ = read_line(r)?; // the CRLF that closes the chunk
    }
}

/// Basic auth needs base64 and nothing else does, so it lives here rather than
/// in a dependency.
pub(crate) fn basic_auth(user: &str, pass: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let raw = format!("{user}:{pass}");
    let mut b64 = String::with_capacity(raw.len().div_ceil(3) * 4);
    for c in raw.as_bytes().chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(c.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(c.get(2).copied().unwrap_or(0));
        for i in 0..4 {
            if i <= c.len() {
                b64.push(T[(n >> (18 - 6 * i)) as usize & 0x3f] as char);
            } else {
                b64.push('=');
            }
        }
    }
    format!("Basic {b64}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648_padding_cases() {
        assert_eq!(super::basic_auth("kodi", "kodi"), "Basic a29kaTprb2Rp");
        assert_eq!(super::basic_auth("a", ""), "Basic YTo=");
        assert_eq!(super::basic_auth("ab", ""), "Basic YWI6");
    }
}
