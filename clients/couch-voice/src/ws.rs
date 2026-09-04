//! A WebSocket client, RFC 6455, client side only.
//!
//! Home Assistant's API is a WebSocket and there is no way around that, so the
//! only question was whether to bring in a crate. `tungstenite` is the good
//! one and it arrives with `rand`, `sha1`, `base64`, `http`, `httparse`,
//! `utf-8` and `byteorder`; the async wrapper drags tokio in behind it. What
//! it buys over the code below is server support, extensions and permessage
//! deflate - none of which is on the path between a remote control and a hub
//! on the same LAN. What it costs is a dependency tree an order of magnitude
//! larger than the crate using it, on a target where every megabyte is flash
//! this device does not have.
//!
//! So: a handshake, six opcodes, and the two primitives underneath it -
//! base64 and SHA-1 - which together are under a hundred lines and are exactly
//! the ones a stripped-down build of those crates would compile anyway.
//!
//! Framing is incremental. A read that times out part way through a frame
//! keeps what it has and returns `None`, because the caller is streaming audio
//! and cannot afford to block on the reply - the same reason the Kodi client's
//! TCP transport counts braces across reads.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// The magic from RFC 6455 section 4.2.2, appended to the client key before
/// hashing so that a cache cannot replay a plain HTTP response as an upgrade.
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// A pipeline's events are small and its audio goes the other way, so anything
/// approaching this is a different protocol or a bug.
const MAX_FRAME: usize = 4 * 1024 * 1024;
const MAX_LINE: u64 = 16 * 1024;

pub const OP_CONTINUATION: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xa;

/// What a socket has to do to carry this. Split out so that `wss://` is a
/// matter of supplying another implementation rather than a rewrite: a TLS
/// stream delegates `set_read_timeout` to the `TcpStream` underneath it.
pub trait Transport: Read + Write + Send {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
}

impl Transport for TcpStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }
}

#[derive(Debug)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    /// The peer went away, cleanly or otherwise. Terminal.
    Closed,
}

pub struct WebSocket {
    stream: Box<dyn Transport>,
    endpoint: String,
    buf: Vec<u8>,
    /// A fragmented message being reassembled, and the opcode it started with.
    partial: Vec<u8>,
    partial_op: u8,
    closed: bool,
    rng: Rng,
}

// The interesting part of a socket is where it points and whether it is still
// there; a derived Debug would print neither and dump four buffers instead.
impl std::fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocket")
            .field("endpoint", &self.endpoint)
            .field("closed", &self.closed)
            .finish()
    }
}

impl WebSocket {
    /// Dial and upgrade. `secure` only changes the scheme in the endpoint
    /// string and the default port; supplying a TLS `Transport` is what would
    /// make it true.
    pub fn connect(host: &str, port: u16, path: &str, timeout: Duration) -> Result<WebSocket> {
        let endpoint = format!("ws://{}{path}", authority(host, port));
        let stream = dial(host, port, timeout, &endpoint)?;
        WebSocket::upgrade(Box::new(stream), host, port, path, endpoint, timeout)
    }

    /// The handshake over an already-connected transport, which is where a
    /// TLS stream would come in.
    pub fn upgrade(
        mut stream: Box<dyn Transport>,
        host: &str,
        port: u16,
        path: &str,
        endpoint: String,
        timeout: Duration,
    ) -> Result<WebSocket> {
        let io_err = |e: io::Error| Error::Io {
            endpoint: endpoint.clone(),
            source: e,
        };
        let bad = |detail: String| Error::Ws {
            endpoint: endpoint.clone(),
            detail,
        };
        stream.set_read_timeout(Some(timeout)).map_err(io_err)?;
        stream.set_write_timeout(Some(timeout)).map_err(io_err)?;

        let mut rng = Rng::new();
        let key = base64(&rng.bytes16());

        let mut head = String::with_capacity(256);
        let _ = write!(head, "GET {path} HTTP/1.1\r\n");
        let _ = write!(head, "Host: {}\r\n", authority(host, port));
        head.push_str("Upgrade: websocket\r\n");
        head.push_str("Connection: Upgrade\r\n");
        let _ = write!(head, "Sec-WebSocket-Key: {key}\r\n");
        head.push_str("Sec-WebSocket-Version: 13\r\n\r\n");
        stream.write_all(head.as_bytes()).map_err(io_err)?;
        stream.flush().map_err(io_err)?;

        // BufReader would swallow bytes of the first frame into a buffer we
        // then throw away, so the response is read a byte at a time. It is one
        // exchange per connection; the syscalls do not matter.
        let mut reader = BufReader::with_capacity(1, &mut stream);
        let status = read_line(&mut reader)
            .map_err(io_err)?
            .ok_or_else(|| bad("it closed the connection without answering".into()))?;
        if !status.starts_with("HTTP/") {
            return Err(bad(format!("expected a status line, got {status:?}")));
        }
        let code: u16 = status
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| bad(format!("unreadable status line {status:?}")))?;

        let mut accept = None;
        loop {
            match read_line(&mut reader).map_err(io_err)? {
                None => return Err(bad("it closed the connection mid-header".into())),
                Some(l) if l.is_empty() => break,
                Some(l) => {
                    if let Some((name, value)) = l.split_once(':') {
                        if name.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                            accept = Some(value.trim().to_string());
                        }
                    }
                }
            }
        }

        if code == 401 || code == 403 {
            // Home Assistant does not authenticate the upgrade - the token
            // goes over the socket - so this is a proxy in front of it.
            return Err(Error::Unauthorized { endpoint });
        }
        if code != 101 {
            return Err(bad(format!(
                "answered HTTP {code} rather than 101; something other than \
                 Home Assistant is on this port, or a proxy is not passing upgrades"
            )));
        }
        let expected = base64(&sha1(format!("{key}{GUID}").as_bytes()));
        match accept {
            Some(got) if got == expected => {}
            Some(got) => {
                return Err(bad(format!(
                    "Sec-WebSocket-Accept was {got:?}, not the {expected:?} this key requires"
                )))
            }
            None => return Err(bad("the 101 carried no Sec-WebSocket-Accept".into())),
        }

        Ok(WebSocket {
            stream,
            endpoint,
            buf: Vec::new(),
            partial: Vec::new(),
            partial_op: 0,
            closed: false,
            rng,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.send(OP_TEXT, text.as_bytes())
    }

    pub fn send_binary(&mut self, data: &[u8]) -> Result<()> {
        self.send(OP_BINARY, data)
    }

    /// The next message, or `None` if none arrived within `timeout`. Pings are
    /// answered here and never surface; a close surfaces once, as `Closed`.
    pub fn recv(&mut self, timeout: Duration) -> Result<Option<Message>> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.take_frame()? {
                Some(frame) => {
                    if let Some(m) = self.handle(frame)? {
                        return Ok(Some(m));
                    }
                    continue;
                }
                None if self.closed => return Ok(Some(Message::Closed)),
                None => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match self.fill(left) {
                Ok(0) => {
                    self.closed = true;
                    return Ok(Some(Message::Closed));
                }
                Ok(_) => {}
                Err(e) if timed_out(&e) => return Ok(None),
                Err(e) => {
                    self.closed = true;
                    return Err(Error::Io {
                        endpoint: self.endpoint.clone(),
                        source: e,
                    });
                }
            }
        }
    }

    /// A courtesy close. Failure is ignored: we are leaving anyway, and the
    /// common case is a peer that has already gone.
    pub fn close(&mut self) {
        if !self.closed {
            let _ = self.send(OP_CLOSE, &1000u16.to_be_bytes());
            self.closed = true;
        }
    }

    // --- internals ----------------------------------------------------------

    fn send(&mut self, opcode: u8, payload: &[u8]) -> Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode); // FIN; nothing here fragments what it sends
        let mask = self.rng.bytes4();
        let n = payload.len();
        // The mask bit is mandatory on every client frame, even an empty one.
        if n < 126 {
            frame.push(0x80 | n as u8);
        } else if n <= u16::MAX as usize {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i & 3]));

        self.stream
            .write_all(&frame)
            .and_then(|()| self.stream.flush())
            .map_err(|e| {
                self.closed = true;
                Error::Io {
                    endpoint: self.endpoint.clone(),
                    source: e,
                }
            })
    }

    fn fill(&mut self, budget: Duration) -> io::Result<usize> {
        // A zero timeout means "block forever" to the socket layer, which is
        // the one thing this must never do.
        self.stream
            .set_read_timeout(Some(budget.max(Duration::from_millis(1))))?;
        let mut chunk = [0u8; 4096];
        let n = self.stream.read(&mut chunk)?;
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }

    /// One complete frame out of the buffer, if there is one.
    fn take_frame(&mut self) -> Result<Option<Frame>> {
        let b = &self.buf;
        if b.len() < 2 {
            return Ok(None);
        }
        let fin = b[0] & 0x80 != 0;
        let opcode = b[0] & 0x0f;
        let masked = b[1] & 0x80 != 0;
        let short = (b[1] & 0x7f) as usize;
        let (len, mut at) = match short {
            126 if b.len() >= 4 => (u16::from_be_bytes([b[2], b[3]]) as usize, 4),
            127 if b.len() >= 10 => (
                u64::from_be_bytes(b[2..10].try_into().unwrap()) as usize,
                10,
            ),
            126 | 127 => return Ok(None),
            n => (n, 2),
        };
        if len > MAX_FRAME {
            return Err(Error::Ws {
                endpoint: self.endpoint.clone(),
                detail: format!("a {len} byte frame is far larger than anything this API sends"),
            });
        }
        // A server must not mask, but handling it is four lines and beats
        // decoding garbage.
        let mask = if masked {
            if b.len() < at + 4 {
                return Ok(None);
            }
            let m = [b[at], b[at + 1], b[at + 2], b[at + 3]];
            at += 4;
            Some(m)
        } else {
            None
        };
        if b.len() < at + len {
            return Ok(None);
        }
        let mut payload = b[at..at + len].to_vec();
        if let Some(m) = mask {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= m[i & 3];
            }
        }
        self.buf.drain(..at + len);
        Ok(Some(Frame {
            fin,
            opcode,
            payload,
        }))
    }

    /// Turn a frame into a message, or into nothing - a ping, a pong, or half
    /// of something still arriving.
    fn handle(&mut self, frame: Frame) -> Result<Option<Message>> {
        match frame.opcode {
            OP_PING => {
                self.send(OP_PONG, &frame.payload)?;
                Ok(None)
            }
            OP_PONG => Ok(None),
            OP_CLOSE => {
                self.closed = true;
                Ok(Some(Message::Closed))
            }
            OP_CONTINUATION => {
                if self.partial_op == 0 {
                    return Err(Error::Ws {
                        endpoint: self.endpoint.clone(),
                        detail: "a continuation frame with nothing to continue".into(),
                    });
                }
                self.partial.extend_from_slice(&frame.payload);
                if !frame.fin {
                    return Ok(None);
                }
                let op = std::mem::take(&mut self.partial_op);
                let data = std::mem::take(&mut self.partial);
                Ok(Some(self.finish(op, data)))
            }
            op @ (OP_TEXT | OP_BINARY) => {
                if frame.fin {
                    return Ok(Some(self.finish(op, frame.payload)));
                }
                self.partial_op = op;
                self.partial = frame.payload;
                Ok(None)
            }
            other => Err(Error::Ws {
                endpoint: self.endpoint.clone(),
                detail: format!("opcode {other:#x} is not one of the six"),
            }),
        }
    }

    fn finish(&self, opcode: u8, data: Vec<u8>) -> Message {
        if opcode == OP_TEXT {
            // Lossy rather than an error: a mangled byte in an event's text is
            // worth surfacing as a question mark, not as a dropped pipeline.
            Message::Text(String::from_utf8_lossy(&data).into_owned())
        } else {
            Message::Binary(data)
        }
    }
}

impl Drop for WebSocket {
    fn drop(&mut self) {
        self.close();
    }
}

struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

// --- dialling ---------------------------------------------------------------

pub fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Connect within `timeout`, whatever the host resolves to, spending the
/// budget across all candidate addresses rather than on each.
fn dial(host: &str, port: u16, timeout: Duration, endpoint: &str) -> Result<TcpStream> {
    use std::net::ToSocketAddrs;
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
                // Audio goes out in period-sized writes on a persistent
                // connection, which is exactly what Nagle delays.
                let _ = s.set_nodelay(true);
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

pub fn timed_out(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn read_line<R: BufRead>(r: &mut R) -> io::Result<Option<String>> {
    let mut raw = Vec::new();
    // Capped: a server that never sends a newline could otherwise grow this
    // until a 1GB device runs out of memory.
    if r.take(MAX_LINE).read_until(b'\n', &mut raw)? == 0 {
        return Ok(None);
    }
    while matches!(raw.last(), Some(b'\n' | b'\r')) {
        raw.pop();
    }
    Ok(Some(String::from_utf8_lossy(&raw).into_owned()))
}

// --- the two primitives the handshake needs ---------------------------------

/// Frame masks and the handshake key. Masking is not a secret - the server
/// unmasks with a key we send it in clear - so this only has to be
/// unpredictable enough that a proxy cannot be poisoned by a crafted payload.
/// The seed comes from the kernel; the stream from an xorshift, so that a
/// socket sending fifty frames a second is not opening `/dev/urandom` fifty
/// times a second.
struct Rng {
    state: u64,
}

impl Rng {
    fn new() -> Rng {
        let mut seed = [0u8; 8];
        let ok = File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut seed))
            .is_ok();
        let mut state = u64::from_le_bytes(seed);
        if !ok || state == 0 {
            // No /dev/urandom is a broken system, but failing to connect over
            // it would be a worse answer than a clock-seeded fallback.
            state = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E3779B97F4A7C15)
                | 1;
        }
        Rng { state }
    }

    fn next(&mut self) -> u64 {
        // xorshift64*, which is three shifts and a multiply.
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn bytes4(&mut self) -> [u8; 4] {
        let n = self.next();
        [n as u8, (n >> 8) as u8, (n >> 16) as u8, (n >> 24) as u8]
    }

    fn bytes16(&mut self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&self.next().to_le_bytes());
        out[8..].copy_from_slice(&self.next().to_le_bytes());
        out
    }
}

/// Standard base64 with padding. Only the handshake needs it, so it lives
/// here rather than in a dependency.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(c.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(c.get(2).copied().unwrap_or(0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i)) as usize & 0x3f] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// SHA-1, needed only to prove the 101 came from something that read our key.
/// Its brokenness as a hash is irrelevant here; RFC 6455 specifies it as a
/// handshake marker, not as security.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());

    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn sha1_matches_the_published_vectors() {
        assert_eq!(
            base64(&sha1(b"abc")),
            base64(&[
                0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50,
                0xc2, 0x6c, 0x9c, 0xd0, 0xd8, 0x9d
            ])
        );
        assert_eq!(
            base64(&sha1(b"")),
            base64(&[
                0xda, 0x39, 0xa3, 0xee, 0x5e, 0x6b, 0x4b, 0x0d, 0x32, 0x55, 0xbf, 0xef, 0x95, 0x60,
                0x18, 0x90, 0xaf, 0xd8, 0x07, 0x09
            ])
        );
        // A message long enough to need a second block, which is where a
        // padding bug hides.
        assert_eq!(
            base64(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            base64(&[
                0x84, 0x98, 0x3e, 0x44, 0x1c, 0x3b, 0xd2, 0x6e, 0xba, 0xae, 0x4a, 0xa1, 0xf9, 0x51,
                0x29, 0xe5, 0xe5, 0x46, 0x70, 0xf1
            ])
        );
    }

    /// The example from RFC 6455 section 1.3, which is the one thing that
    /// proves the handshake and the two primitives agree with the world.
    #[test]
    fn the_rfcs_own_handshake_example_checks_out() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        assert_eq!(
            base64(&sha1(format!("{key}{GUID}").as_bytes())),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn base64_pads_the_way_rfc4648_says() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// A whole conversation against a socket pretending to be Home Assistant:
    /// the upgrade, a text frame, a masked binary frame going the other way,
    /// a ping that must be answered without surfacing, and a fragmented
    /// message. All the framing rules that a real server would exercise once
    /// and that are impossible to debug remotely.
    #[test]
    fn frames_survive_a_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut byte = [0u8; 1];
            while !req.ends_with(b"\r\n\r\n") {
                s.read_exact(&mut byte).unwrap();
                req.push(byte[0]);
            }
            let req = String::from_utf8(req).unwrap();
            let key = req
                .lines()
                .find_map(|l| l.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap()
                .trim()
                .to_string();
            let accept = base64(&sha1(format!("{key}{GUID}").as_bytes()));
            s.write_all(
                format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
                     Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                )
                .as_bytes(),
            )
            .unwrap();

            // Unmasked server frames, glued together: a ping, a whole text
            // message, and a text message split across two frames.
            let mut out = Vec::new();
            out.extend_from_slice(&[0x89, 0x02, b'h', b'i']); // ping
            out.extend_from_slice(&[0x81, 0x05]);
            out.extend_from_slice(b"first");
            out.extend_from_slice(&[0x01, 0x03]); // text, not final
            out.extend_from_slice(b"se+");
            out.extend_from_slice(&[0x80, 0x04]); // continuation, final
            out.extend_from_slice(b"cond");
            s.write_all(&out).unwrap();

            // Then read everything the client sends until it hangs up: a
            // pong, a text and a binary. Reading to EOF rather than to a byte
            // count, because a count is a deadlock waiting for a framing bug.
            let mut got = Vec::new();
            let mut chunk = [0u8; 256];
            loop {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => got.extend_from_slice(&chunk[..n]),
                }
            }
            got
        });

        let mut ws =
            WebSocket::connect("127.0.0.1", port, "/api/websocket", Duration::from_secs(2))
                .unwrap();
        let first = ws.recv(Duration::from_secs(2)).unwrap().unwrap();
        assert!(matches!(first, Message::Text(t) if t == "first"));
        // The ping was answered and never surfaced; the fragments arrived as
        // one message.
        let second = ws.recv(Duration::from_secs(2)).unwrap().unwrap();
        assert!(matches!(second, Message::Text(t) if t == "se+cond"));
        assert!(ws.recv(Duration::from_millis(50)).unwrap().is_none());

        ws.send_text("ping").unwrap();
        ws.send_binary(&[1, 2, 3]).unwrap();
        drop(ws); // so the server's read reaches EOF rather than blocking
        let got = server.join().unwrap();
        // A pong (0x8a) came first, then the two frames, all client-masked.
        assert_eq!(got[0], 0x8a);
        assert_eq!(got[1] & 0x80, 0x80, "client frames must be masked");
    }

    #[test]
    fn an_upgrade_that_is_not_one_says_so() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut scratch = [0u8; 1024];
            let _ = s.read(&mut scratch);
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        });
        let e = WebSocket::connect("127.0.0.1", port, "/api/websocket", Duration::from_secs(2))
            .unwrap_err();
        assert!(e.to_string().contains("404"), "{e}");
    }
}
