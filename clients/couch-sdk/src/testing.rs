//! Building a client without the device, and proving it without the device.
//!
//! Two things live here. [`MockHost`] is a scripted TCP peer: it answers the
//! line-oriented request/response pattern that the Denon, Kodi and webOS
//! transports all use, and it can also refuse to answer or hang up, which are
//! the two failure paths a client gets wrong. [`contract_findings`] runs a
//! client against whatever host you point it at and reports what it got wrong.
//!
//! Neither needs hardware, a credential, or a network beyond loopback, which is
//! the whole point: a contributor with no Astrion HA100 on their desk can still
//! write a client and know it is correct up to the wire format.
//!
//! Enable with `features = ["testing"]` in `[dev-dependencies]`.

use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use couch_model::commands::Function;

use crate::{ClientSettings, DeviceClient, Error};

/// How the fake host answers one request.
#[derive(Clone, Debug)]
pub enum Reply {
    /// One line, plus the terminator.
    Line(String),
    /// Several lines, written in one `write_all` so the client also sees the
    /// case where two replies arrive in a single read.
    Lines(Vec<String>),
    /// Say nothing at all. The client must time out rather than block forever.
    Silence,
    /// Hang up mid-conversation.
    Close,
}

impl Reply {
    pub fn line(text: impl Into<String>) -> Self {
        Self::Line(text.into())
    }
}

/// What the fake host says, and to what.
#[derive(Clone, Debug)]
pub struct Script {
    terminator: u8,
    rules: Vec<(String, Reply)>,
    fallback: Reply,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            terminator: b'\r',
            rules: Vec::new(),
            fallback: Reply::Silence,
        }
    }
}

impl Script {
    pub fn new() -> Self {
        Self::default()
    }

    /// The byte that ends a request and each reply line. `\r` for a Denon AVR,
    /// `\n` for a newline protocol.
    pub fn terminator(mut self, byte: u8) -> Self {
        self.terminator = byte;
        self
    }

    /// Answer `request` with `reply`. Rules are matched in the order added;
    /// adding the same request twice answers it differently on each occurrence,
    /// which is how you script a state change.
    pub fn on(mut self, request: impl Into<String>, reply: Reply) -> Self {
        self.rules.push((request.into(), reply));
        self
    }

    /// What to do with a request no rule matched. Defaults to [`Reply::Silence`].
    pub fn otherwise(mut self, reply: Reply) -> Self {
        self.fallback = reply;
        self
    }
}

/// A scripted device on loopback.
///
/// Stops when dropped. Every request it received is available from
/// [`MockHost::requests`], so a test can assert that a refused command cost no
/// round trip at all.
pub struct MockHost {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl MockHost {
    pub fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let port = listener.local_addr().expect("local address").port();
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let host = Self {
            port,
            requests: requests.clone(),
            stop: stop.clone(),
        };
        std::thread::spawn(move || {
            let mut sessions = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        let script = script.clone();
                        let requests = requests.clone();
                        let stop = stop.clone();
                        sessions.push(std::thread::spawn(move || {
                            serve(socket, script, requests, stop)
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => return,
                }
            }
            for session in sessions {
                let _ = session.join();
            }
        });
        host
    }

    pub fn host(&self) -> &'static str {
        "127.0.0.1"
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Every complete request received so far, oldest first.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("request log").clone()
    }
}

impl Drop for MockHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn serve(
    mut socket: TcpStream,
    script: Script,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
) {
    let _ = socket.set_read_timeout(Some(Duration::from_millis(50)));
    let mut pending: VecDeque<(String, Reply)> = script.rules.into_iter().collect();
    let mut buffer: Vec<u8> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        let mut bytes = [0; 256];
        match socket.read(&mut bytes) {
            Ok(0) => return,
            Ok(n) => buffer.extend_from_slice(&bytes[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => return,
        }
        while let Some(at) = buffer.iter().position(|b| *b == script.terminator) {
            let line: Vec<u8> = buffer.drain(..=at).collect();
            let Ok(request) = String::from_utf8(line[..at].to_vec()) else {
                return;
            };
            requests.lock().expect("request log").push(request.clone());
            let reply = match pending.iter().position(|(want, _)| *want == request) {
                Some(index) => pending.remove(index).expect("scripted reply").1,
                None => script.fallback.clone(),
            };
            let lines = match reply {
                Reply::Line(line) => vec![line],
                Reply::Lines(lines) => lines,
                Reply::Silence => continue,
                Reply::Close => return,
            };
            let mut out = Vec::new();
            for line in lines {
                out.extend_from_slice(line.as_bytes());
                out.push(script.terminator);
            }
            if socket.write_all(&out).is_err() {
                return;
            }
        }
    }
}

/// Everything wrong with a client, as a list of sentences.
///
/// Empty means the client keeps the contract: its declarations parse, it
/// refuses what it never declared, it refuses nonsense without touching the
/// network, and its settings survive being written to disk and read back.
///
/// `settings` must address a reachable host - in a test, a [`MockHost`].
pub fn contract_findings<C: DeviceClient>(settings: &C::Settings) -> Vec<String> {
    let mut findings = Vec::new();
    if C::KIND.is_empty()
        || !C::KIND
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        findings.push(format!(
            "KIND {:?} must be a lowercase kebab-case provider slug, matching Provider::kind()",
            C::KIND
        ));
    }
    if C::LABEL.is_empty() {
        findings.push("LABEL is empty; the connections UI has nothing to show".into());
    }
    if C::capabilities().is_empty() {
        findings
            .push("capabilities() is empty; no button could ever be mapped to this device".into());
    }
    for (index, (id, label)) in C::capabilities().iter().enumerate() {
        if label.is_empty() {
            findings.push(format!("capability `{id}` has no label"));
        }
        if C::capabilities()[..index]
            .iter()
            .any(|(seen, _)| seen == id)
        {
            findings.push(format!("capability `{id}` is declared twice"));
        }
        if id.starts_with("input:") || id.starts_with("app:") {
            findings.push(format!(
                "capability `{id}` is a dynamic function; declare it with supports_input/supports_app instead"
            ));
            continue;
        }
        match Function::parse(id) {
            Some(function) if function.id() == *id => {
                if !C::supports(&function) {
                    findings.push(format!(
                        "capability `{id}` is declared but supports() refuses it"
                    ));
                }
            }
            Some(function) => findings.push(format!(
                "capability `{id}` is not canonical; couch_model spells it `{}`",
                function.id()
            )),
            None => findings.push(format!(
                "capability `{id}` is not a couch_model::commands::Function"
            )),
        }
    }
    if let Err(e) = settings.validate() {
        findings.push(format!("the settings given to this check are invalid: {e}"));
        return findings;
    }
    match roundtrip_settings::<C>(settings) {
        Ok(()) => {}
        Err(e) => findings.push(e),
    }
    let mut client = match C::connect(settings) {
        Ok(client) => client,
        Err(e) => {
            findings.push(format!("connect failed against the given host: {e}"));
            return findings;
        }
    };
    if client.command("this-is-not-a-function") != Err(Error::Unsupported) {
        findings.push(
            "command() must answer Unsupported for text that is not a Function, without any I/O"
                .into(),
        );
    }
    if let Some(undeclared) = ["power-off", "mute", "stop", "home", "yellow"]
        .into_iter()
        .find(|id| !C::capabilities().iter().any(|(name, _)| name == id))
    {
        if client.command(undeclared) != Err(Error::Unsupported) {
            findings.push(format!(
                "`{undeclared}` is not declared, so command() must answer Unsupported without contacting the device"
            ));
        }
    }
    // Short-circuits before any I/O for a client that does support apps: this
    // check is about the gate, not about launching something on a real device.
    if !C::supports_app("definitely-not-installed")
        && client.command("app:definitely-not-installed") == Ok(())
    {
        findings.push("command() ran an app: function that supports_app refuses".into());
    }
    findings
}

fn roundtrip_settings<C: DeviceClient>(settings: &C::Settings) -> std::result::Result<(), String> {
    let dir = std::env::temp_dir().join(format!(
        "couch-sdk-contract-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let connection = "contract-check";
    std::fs::create_dir_all(dir.join("connections").join(connection))
        .map_err(|e| format!("cannot create a temporary connection directory: {e}"))?;
    let path = C::Settings::path_in(&dir, connection);
    let result = (|| {
        settings
            .save(&path)
            .map_err(|e| format!("settings could not be saved to {}: {e}", path.display()))?;
        let loaded = C::Settings::load(&path)
            .map_err(|e| format!("settings could not be read back from disk: {e}"))?;
        if serde_json::to_value(&loaded).ok() != serde_json::to_value(settings).ok() {
            return Err("settings did not survive a save/load round trip unchanged".into());
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// [`contract_findings`], as an assertion. Panics listing everything at once.
pub fn assert_contract<C: DeviceClient>(settings: &C::Settings) {
    let findings = contract_findings::<C>(settings);
    assert!(
        findings.is_empty(),
        "{} does not keep the client contract:\n- {}",
        C::KIND,
        findings.join("\n- ")
    );
}
