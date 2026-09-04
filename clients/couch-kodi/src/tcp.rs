//! Kodi's raw TCP JSON-RPC transport - port 9090, and the primary one.
//!
//! Three things make it the right default over the HTTP interface. It is on by
//! default, where the web server has to be switched on by hand; the connection
//! is persistent, so a keypress costs one write rather than a handshake; and
//! Kodi pushes notifications down it unasked - Player.OnPlay, OnPause,
//! Application.OnVolumeChanged - which is what lets the UI stop polling.
//!
//! There is no framing. Objects are written onto the socket back to back, and
//! replies interleave with those pushed notifications, so the reader has to
//! find object boundaries by counting braces itself and sort what it finds by
//! whether it carries an `id`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::net;

/// A single object this size is a different protocol or a bug, and buffering it
/// on a 1GB device to find that out is not worth it.
const MAX_OBJECT: usize = 8 * 1024 * 1024;

/// Notifications pile up while nobody is reading them. A remote that has been
/// sitting on a menu for an hour should not be holding an hour of Kodi's
/// chatter; the newest events are the ones that describe the current state.
const MAX_QUEUED: usize = 256;

pub(crate) struct Tcp {
    pub host: String,
    pub port: u16,
    pub timeout: Duration,
    /// One connection, driven from one thread. A Mutex here would let a daemon
    /// park a second thread in next_notification and silently block every call
    /// behind it, which is worse than not being Sync.
    state: RefCell<State>,
}

#[derive(Default)]
struct State {
    stream: Option<TcpStream>,
    buf: Vec<u8>,
    scan: Scan,
    queue: VecDeque<Value>,
}

/// Where the brace counter had got to. Kept between reads so a half-arrived
/// object is not rescanned from the start every time more bytes turn up.
#[derive(Default, Clone, Copy)]
struct Scan {
    at: usize,
    start: usize,
    depth: usize,
    in_string: bool,
    escape: bool,
}

impl Tcp {
    pub fn new(host: String, port: u16, timeout: Duration) -> Self {
        Tcp {
            host,
            port,
            timeout,
            state: RefCell::new(State::default()),
        }
    }

    pub fn endpoint(&self) -> String {
        format!("tcp://{}", net::authority(&self.host, self.port))
    }

    /// Send a request and wait for the reply with the matching id, queueing any
    /// notification that arrives while we wait.
    pub fn exchange(&self, request: &Value, id: u64) -> Result<Value> {
        let mut body = serde_json::to_vec(request)?;
        // Kodi is happy with objects back to back; the newline is a courtesy to
        // anyone watching the socket with nc.
        body.push(b'\n');

        match self.attempt(&body, id) {
            Ok(v) => Ok(v),
            Err((e, retry)) => {
                self.forget();
                if !retry {
                    return Err(e);
                }
                // Kodi was restarted and we were holding a dead socket. One
                // reconnect, one retry: the request never reached it, so even
                // the toggles are safe to send again.
                self.attempt(&body, id).map_err(|(e, _)| {
                    self.forget();
                    e
                })
            }
        }
    }

    /// The next pushed notification, or `None` if none arrived in `timeout`.
    pub fn next_notification(&self, timeout: Duration) -> Result<Option<Value>> {
        let deadline = Instant::now() + timeout;
        let mut st = self.state.borrow_mut();
        if let Some(v) = st.queue.pop_front() {
            return Ok(Some(v));
        }
        // Nothing is pushed to a client that is not connected, so a watch on a
        // handle that has only ever been idle has to dial first.
        self.connect(&mut st)?;

        loop {
            match self.next_object(&mut st)? {
                Some(v) if v.get("id").is_none() => return Ok(Some(v)),
                // A reply to a call that has already given up. Nobody is
                // waiting for it.
                Some(_) => continue,
                None => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match st.fill(left) {
                Ok(0) => {
                    st.stream = None;
                    return Err(self.io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Kodi closed the connection",
                    )));
                }
                Ok(_) => {}
                Err(e) if net::timed_out(&e) => return Ok(None),
                Err(e) => {
                    st.stream = None;
                    return Err(self.io(e));
                }
            }
        }
    }

    // --- internals ----------------------------------------------------------

    /// `Err`'s second member is whether the failure is worth one retry on a
    /// fresh connection.
    fn attempt(&self, body: &[u8], id: u64) -> std::result::Result<Value, (Error, bool)> {
        let deadline = Instant::now() + self.timeout;
        let mut st = self.state.borrow_mut();

        let reused = st.stream.is_some();
        self.connect(&mut st).map_err(|e| (e, false))?;

        // Writing onto a socket Kodi has since closed is the classic stale
        // handle, and only worth retrying if the socket was one we had been
        // holding rather than one we just opened.
        if let Err(e) = st.write(body, self.timeout) {
            return Err((self.io(e), reused));
        }

        let mut heard = false;
        loop {
            match self.next_object(&mut st) {
                Err(e) => return Err((e, false)),
                Ok(Some(v)) => {
                    match v.get("id") {
                        // No id: a pushed notification, not our reply.
                        None => st.enqueue(v),
                        // Kodi answers a request it could not parse with a null
                        // id. Only one request is ever in flight on this
                        // socket, so that is still ours.
                        Some(Value::Null) => return Ok(v),
                        Some(rid) if rid.as_u64() == Some(id) => return Ok(v),
                        // A reply to a call that timed out earlier.
                        Some(_) => {}
                    }
                    continue;
                }
                Ok(None) => {}
            }

            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let e = io::Error::new(io::ErrorKind::TimedOut, "no reply");
                return Err((self.io(e), false));
            }
            match st.fill(left) {
                // EOF before a single byte came back means Kodi never saw the
                // request; once anything has arrived it may well have acted on
                // it, and re-sending a PlayPause would undo itself.
                Ok(0) => {
                    let e =
                        io::Error::new(io::ErrorKind::UnexpectedEof, "Kodi closed the connection");
                    return Err((self.io(e), reused && !heard));
                }
                Ok(_) => heard = true,
                Err(e) => return Err((self.io(e), false)),
            }
        }
    }

    fn connect(&self, st: &mut State) -> Result<()> {
        if st.stream.is_some() {
            return Ok(());
        }
        let stream = net::connect(&self.host, self.port, self.timeout, &self.endpoint())?;
        // Small writes on a persistent connection are exactly what Nagle
        // delays, and a keypress that waits 40ms for company feels it.
        let _ = stream.set_nodelay(true);
        st.buf.clear();
        st.scan = Scan::default();
        st.stream = Some(stream);
        Ok(())
    }

    fn forget(&self) {
        let mut st = self.state.borrow_mut();
        st.stream = None;
        st.buf.clear();
        st.scan = Scan::default();
    }

    fn next_object(&self, st: &mut State) -> Result<Option<Value>> {
        let bad = |detail: String| Error::Protocol {
            endpoint: self.endpoint(),
            detail,
        };
        let Some(raw) = st.take_object().map_err(bad)? else {
            return Ok(None);
        };
        serde_json::from_slice(&raw).map(Some).map_err(|e| {
            bad(format!(
                "{e} (object began {:?})",
                String::from_utf8_lossy(&raw[..raw.len().min(72)])
            ))
        })
    }

    fn io(&self, source: io::Error) -> Error {
        Error::Io {
            endpoint: self.endpoint(),
            source,
        }
    }
}

impl State {
    /// The next complete top-level object, by counting braces outside strings.
    fn take_object(&mut self) -> std::result::Result<Option<Vec<u8>>, String> {
        if self.scan.depth == 0 {
            while self.scan.at < self.buf.len() && self.buf[self.scan.at].is_ascii_whitespace() {
                self.scan.at += 1;
            }
            if self.scan.at >= self.buf.len() {
                self.buf.drain(..self.scan.at);
                self.scan.at = 0;
                return Ok(None);
            }
            if self.buf[self.scan.at] != b'{' {
                let seen = String::from_utf8_lossy(
                    &self.buf[self.scan.at..self.buf.len().min(self.scan.at + 40)],
                )
                .into_owned();
                return Err(format!("expected a JSON object, got {seen:?}"));
            }
            self.scan.start = self.scan.at;
        }

        while self.scan.at < self.buf.len() {
            let b = self.buf[self.scan.at];
            self.scan.at += 1;
            if self.scan.in_string {
                // Only the escape and the closing quote matter; a multi-byte
                // UTF-8 sequence can never collide with either, so scanning
                // bytes rather than chars is safe here.
                if self.scan.escape {
                    self.scan.escape = false;
                } else if b == b'\\' {
                    self.scan.escape = true;
                } else if b == b'"' {
                    self.scan.in_string = false;
                }
                continue;
            }
            match b {
                b'"' => self.scan.in_string = true,
                b'{' => self.scan.depth += 1,
                b'}' => {
                    self.scan.depth -= 1;
                    if self.scan.depth == 0 {
                        let end = self.scan.at;
                        let obj = self.buf[self.scan.start..end].to_vec();
                        self.buf.drain(..end);
                        self.scan = Scan::default();
                        return Ok(Some(obj));
                    }
                }
                _ => {}
            }
        }
        if self.buf.len() > MAX_OBJECT {
            return Err(format!("a single object has run past {MAX_OBJECT} bytes"));
        }
        Ok(None)
    }

    fn enqueue(&mut self, v: Value) {
        if self.queue.len() == MAX_QUEUED {
            self.queue.pop_front();
        }
        self.queue.push_back(v);
    }

    fn write(&mut self, body: &[u8], timeout: Duration) -> io::Result<()> {
        let stream = self.stream.as_mut().ok_or(io::ErrorKind::NotConnected)?;
        stream.set_write_timeout(Some(timeout))?;
        stream.write_all(body)?;
        stream.flush()
    }

    fn fill(&mut self, budget: Duration) -> io::Result<usize> {
        let stream = self.stream.as_mut().ok_or(io::ErrorKind::NotConnected)?;
        // A zero duration means "block forever" to the socket layer, which is
        // the one thing this client must never do.
        stream.set_read_timeout(Some(budget.max(Duration::from_millis(1))))?;
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk)?;
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::{State, Tcp};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    fn objects(feed: &[&str]) -> Vec<String> {
        let mut st = State::default();
        let mut out = Vec::new();
        for part in feed {
            st.buf.extend_from_slice(part.as_bytes());
            while let Some(o) = st.take_object().unwrap() {
                out.push(String::from_utf8(o).unwrap());
            }
        }
        out
    }

    #[test]
    fn splits_objects_that_arrive_glued_together() {
        assert_eq!(
            objects(&[r#"{"id":1}{"id":2}"#]),
            vec![r#"{"id":1}"#, r#"{"id":2}"#]
        );
    }

    #[test]
    fn waits_for_an_object_split_across_reads() {
        assert_eq!(
            objects(&[r#"{"a":{"b":"#, r#"1}}"#]),
            vec![r#"{"a":{"b":1}}"#]
        );
    }

    #[test]
    fn braces_inside_strings_do_not_count() {
        let s = r#"{"title":"a } b { c","esc":"\"}\\"}"#;
        assert_eq!(objects(&[s]), vec![s]);
    }

    #[test]
    fn whitespace_between_objects_is_skipped() {
        assert_eq!(
            objects(&["{\"a\":1}\n \r\n{\"b\":2}"]),
            vec![r#"{"a":1}"#, r#"{"b":2}"#]
        );
    }

    /// The contract that matters most: a push landing in front of the reply
    /// must neither be mistaken for the reply nor thrown away.
    #[test]
    fn notifications_around_a_call_are_queued_not_dropped() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut scratch = [0u8; 512];
            let _request = s.read(&mut scratch).unwrap();
            // Glued together, and the reply is sandwiched between two pushes.
            s.write_all(
                br#"{"jsonrpc":"2.0","method":"Player.OnPause","params":{"sender":"xbmc"}}"#,
            )
            .unwrap();
            s.write_all(br#"{"id":1,"jsonrpc":"2.0","result":"pong"}"#)
                .unwrap();
            s.write_all(
                br#"{"jsonrpc":"2.0","method":"Player.OnResume","params":{"sender":"xbmc"}}"#,
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });

        let t = Tcp::new("127.0.0.1".into(), port, Duration::from_secs(2));
        let request = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"JSONRPC.Ping"});
        let reply = t.exchange(&request, 1).unwrap();
        assert_eq!(reply["result"], "pong");

        let first = t
            .next_notification(Duration::from_millis(500))
            .unwrap()
            .unwrap();
        assert_eq!(first["method"], "Player.OnPause");
        let second = t
            .next_notification(Duration::from_millis(500))
            .unwrap()
            .unwrap();
        assert_eq!(second["method"], "Player.OnResume");
        assert!(t
            .next_notification(Duration::from_millis(50))
            .unwrap()
            .is_none());
        server.join().unwrap();
    }

    #[test]
    fn anything_but_an_object_is_refused() {
        let mut st = State::default();
        st.buf.extend_from_slice(b"SSH-2.0-OpenSSH_9.6\r\n");
        assert!(st.take_object().is_err());
    }
}
