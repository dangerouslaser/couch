//! Denon AVR Ethernet control. A single owner handles CR-delimited replies and
//! unsolicited events. Commands are never automatically retried.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};
pub const DEFAULT_PORT: u16 = 23;
pub const TIMEOUT: Duration = Duration::from_secs(2);
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Invalid,
    Timeout,
    Protocol,
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if matches!(
            e.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            Self::Timeout
        } else {
            Self::Io(e)
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io(_) => "Cannot reach the AVR. Check its address and Network Control setting.",
            Self::Invalid => "Invalid AVR command or setting",
            Self::Timeout => "AVR did not confirm the request before the deadline",
            Self::Protocol => "Invalid AVR response",
        })
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub host: String,
    pub port: u16,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.host.is_empty()
            || self.host.len() > 253
            || self
                .host
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '\\'))
            || self.port == 0
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let s: Self = serde_json::from_slice(&std::fs::read(path)?).map_err(|_| Error::Invalid)?;
        s.validate()?;
        Ok(s)
    }
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        self.validate()?;
        let tmp = path.with_extension(format!("{}.new", std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(&serde_json::to_vec(self)?)?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)?;
            std::fs::File::open(path.parent().unwrap())?.sync_all()
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(tmp);
        }
        result.map_err(Into::into)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct State {
    pub on: Option<bool>,
    pub muted: Option<bool>,
    pub volume_db: Option<f32>,
    pub volume_minimum: bool,
    pub input: Option<String>,
}
#[derive(Clone, Debug)]
pub enum Command {
    Power(bool),
    Mute(bool),
    VolumeUp,
    VolumeDown,
    VolumeDb(f32),
    Input(String),
}
fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 25
        && value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b" /+-".contains(&b))
}
fn volume(db: f32) -> Result<String> {
    if !db.is_finite() || !(-80.0..=18.0).contains(&db) || (db * 2.0).fract() != 0.0 {
        return Err(Error::Invalid);
    }
    let value = ((db + 80.0) * 10.0).round() as u16;
    Ok(if value % 10 == 0 {
        format!("MV{:02}", value / 10)
    } else {
        format!("MV{value:03}")
    })
}
impl Command {
    fn wire(&self) -> Result<String> {
        Ok(match self {
            Self::Power(on) => if *on { "ZMON" } else { "ZMOFF" }.into(),
            Self::Mute(on) => if *on { "MUON" } else { "MUOFF" }.into(),
            Self::VolumeUp => "MVUP".into(),
            Self::VolumeDown => "MVDOWN".into(),
            Self::VolumeDb(db) => return volume(*db),
            Self::Input(id) if token(id) => format!("SI{id}"),
            _ => return Err(Error::Invalid),
        })
    }
}
/// Recognize state updates while ignoring MVMAX, surround/channel data and
/// unknown extensions. Minimum volume is not a fabricated numeric reading.
pub fn apply(state: &mut State, line: &str) -> Option<&'static str> {
    match line {
        "ZMON" => {
            state.on = Some(true);
            Some("ZM")
        }
        "ZMOFF" => {
            state.on = Some(false);
            Some("ZM")
        }
        "MUON" => {
            state.muted = Some(true);
            Some("MU")
        }
        "MUOFF" => {
            state.muted = Some(false);
            Some("MU")
        }
        _ if line.starts_with("MV") => {
            let v = &line[2..];
            if !(v.len() == 2 || v.len() == 3) || !v.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let n = v.parse::<u16>().ok()?;
            let tenths = if v.len() == 2 { n * 10 } else { n };
            if tenths > 980 || tenths % 5 != 0 {
                return None;
            }
            state.volume_minimum = tenths == 0;
            state.volume_db = (tenths != 0).then_some(tenths as f32 / 10.0 - 80.0);
            Some("MV")
        }
        _ if line.starts_with("SI") && token(&line[2..]) => {
            state.input = Some(line[2..].into());
            Some("SI")
        }
        _ => None,
    }
}
pub struct Client {
    socket: TcpStream,
    buffer: Vec<u8>,
    pub state: State,
    send_after: Instant,
}
impl Client {
    pub fn connect(settings: &Settings) -> Result<Self> {
        settings.validate()?;
        let deadline = Instant::now() + TIMEOUT;
        let mut socket = None;
        for address in (settings.host.as_str(), settings.port).to_socket_addrs()? {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            if let Ok(s) = TcpStream::connect_timeout(&address, left) {
                socket = Some(s);
                break;
            }
        }
        let socket = socket.ok_or(Error::Timeout)?;
        socket.set_write_timeout(Some(TIMEOUT))?;
        socket.set_nodelay(true)?;
        Ok(Self {
            socket,
            buffer: Vec::new(),
            state: State::default(),
            send_after: Instant::now(),
        })
    }
    fn send(&mut self, line: &str) -> Result<()> {
        std::thread::sleep(self.send_after.saturating_duration_since(Instant::now()));
        self.socket.write_all(line.as_bytes())?;
        self.socket.write_all(b"\r")?;
        self.send_after = Instant::now() + Duration::from_millis(50);
        Ok(())
    }
    fn line(&mut self, deadline: Instant) -> Result<String> {
        loop {
            if let Some(at) = self.buffer.iter().position(|b| *b == b'\r') {
                if at > 1024 {
                    return Err(Error::Protocol);
                }
                let line: Vec<_> = self.buffer.drain(..=at).collect();
                let line = std::str::from_utf8(&line[..at])
                    .map_err(|_| Error::Protocol)?
                    .trim_start_matches('\n')
                    .to_string();
                if !line.is_empty() {
                    return Ok(line);
                }
                continue;
            }
            if self.buffer.len() > 1024 {
                return Err(Error::Protocol);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout);
            }
            self.socket.set_read_timeout(Some(left))?;
            let mut bytes = [0; 512];
            let n = self.socket.read(&mut bytes)?;
            if n == 0 {
                return Err(Error::Io(io::Error::from(io::ErrorKind::UnexpectedEof)));
            }
            self.buffer.extend_from_slice(&bytes[..n]);
        }
    }
    fn drain(&mut self) -> Result<()> {
        let until = Instant::now() + Duration::from_millis(10);
        loop {
            match self.line(until) {
                Ok(line) => {
                    apply(&mut self.state, &line);
                }
                Err(Error::Timeout) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }
    fn query(&mut self, prefix: &str) -> Result<()> {
        self.drain()?;
        self.send(&format!("{prefix}?"))?;
        let until = Instant::now() + TIMEOUT;
        loop {
            let line = self.line(until)?;
            if apply(&mut self.state, &line) == Some(prefix) {
                return Ok(());
            }
        }
    }
    /// Input IDs and user-assigned source names, terminated by SSFUN END.
    pub fn sources(&mut self) -> Result<Vec<(String, String)>> {
        self.drain()?;
        self.send("SSFUN ?")?;
        let until = Instant::now() + TIMEOUT;
        let mut sources = Vec::new();
        loop {
            let line = self.line(until)?;
            apply(&mut self.state, &line);
            if line == "SSFUN END" {
                return Ok(sources);
            }
            if let Some(value) = line.strip_prefix("SSFUN") {
                if let Some((id, name)) = value.split_once(' ') {
                    if token(id) && !name.is_empty() {
                        sources.push((id.into(), name.trim().into()));
                        if sources.len() > 100 {
                            return Err(Error::Protocol);
                        }
                    }
                }
            }
        }
    }
    pub fn status(&mut self) -> Result<State> {
        for key in ["ZM", "MV", "MU", "SI"] {
            self.query(key)?;
        }
        Ok(self.state.clone())
    }
    pub fn command(&mut self, command: Command) -> Result<State> {
        let line = command.wire()?;
        self.drain()?;
        self.send(&line)?;
        if matches!(command, Command::Power(true)) {
            self.send_after = Instant::now() + Duration::from_secs(1);
        }
        // Consume command events, then ask for a fresh observation. Absolute
        // commands only succeed when the receiver confirms the requested value.
        self.query(&line[..2])?;
        let confirmed = match &command {
            Command::Power(on) => self.state.on == Some(*on),
            Command::Mute(on) => self.state.muted == Some(*on),
            Command::VolumeDb(db) => {
                self.state.volume_db == Some(*db) || *db == -80.0 && self.state.volume_minimum
            }
            Command::Input(id) => self.state.input.as_ref() == Some(id),
            _ => true,
        };
        if !confirmed {
            return Err(Error::Protocol);
        }
        Ok(self.state.clone())
    }
    /// Receive an unsolicited update without a polling request. The owner can
    /// fall back to `status` after reconnect; never share a socket across threads.
    pub fn next_event(&mut self, wait: Duration) -> Result<Option<State>> {
        let until = Instant::now() + wait;
        loop {
            match self.line(until) {
                Ok(line) => {
                    if apply(&mut self.state, &line).is_some() {
                        return Ok(Some(self.state.clone()));
                    }
                }
                Err(Error::Timeout) => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_half_steps_and_extensions_are_unambiguous() {
        let mut s = State::default();
        assert_eq!(apply(&mut s, "MV455"), Some("MV"));
        assert_eq!(s.volume_db, Some(-34.5));
        assert_eq!(apply(&mut s, "MVMAX 91"), None);
        assert_eq!(s.volume_db, Some(-34.5));
        apply(&mut s, "MV00");
        assert!(s.volume_minimum);
        assert_eq!(s.volume_db, None);
        assert_eq!(volume(-34.5).unwrap(), "MV455");
        assert_eq!(volume(0.0).unwrap(), "MV80");
        for bad in [f32::NAN, 19.0, -81.0, 0.2] {
            assert!(volume(bad).is_err());
        }
        assert!(Command::Input("BD\rMV98".into()).wire().is_err());
    }
    #[test]
    fn fragmented_push_frames_do_not_confuse_status_queries() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut b = [0; 1];
            for (request, response) in [
                ("ZM?", "MV455\rZMON\r"),
                ("MV?", "MVMAX 91\rMV455\r"),
                ("MU?", "MUOFF\r"),
                ("SI?", "SIBD\r"),
            ] {
                let mut line = Vec::new();
                loop {
                    socket.read_exact(&mut b).unwrap();
                    if b[0] == b'\r' {
                        break;
                    }
                    line.push(b[0]);
                }
                assert_eq!(String::from_utf8(line).unwrap(), request);
                for chunk in response.as_bytes().chunks(2) {
                    socket.write_all(chunk).unwrap();
                }
            }
        });
        let mut client = Client::connect(&Settings {
            host: "127.0.0.1".into(),
            port,
        })
        .unwrap();
        let s = client.status().unwrap();
        assert_eq!(s.input.as_deref(), Some("BD"));
        assert_eq!(s.volume_db, Some(-34.5));
        assert_eq!(s.on, Some(true));
        server.join().unwrap();
    }
}
