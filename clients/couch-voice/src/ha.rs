//! Home Assistant's Assist pipeline, over its WebSocket API.
//!
//! The exchange, as Home Assistant actually implements it
//! (`homeassistant/components/assist_pipeline/websocket_api.py`):
//!
//! ```text
//! remote                                   Home Assistant
//!   |  <- {"type":"auth_required","ha_version":...}
//!   |  -> {"type":"auth","access_token":"..."}
//!   |  <- {"type":"auth_ok"}        or auth_invalid, and it hangs up
//!   |  -> {"id":1,"type":"assist_pipeline/run","start_stage":"stt",
//!   |      "end_stage":"intent","input":{"sample_rate":16000}}
//!   |  <- {"id":1,"type":"result","success":true}
//!   |  <- event run-start   -> runner_data.stt_binary_handler_id = N
//!   |  <- event stt-start
//!   |  => BINARY frame: [N][...raw PCM...]      repeatedly, as it is captured
//!   |  <- event stt-vad-start / stt-vad-end     (server-side voice activity)
//!   |  <- event stt-end     -> stt_output.text  <- the transcription
//!   |  => BINARY frame: [N]                     one byte, end of stream
//!   |  <- event intent-start / intent-end       -> the assistant's answer
//!   |  <- event run-end
//! ```
//!
//! Three details are not obvious from the documentation and cost a day each
//! if you get them wrong.
//!
//! **The audio format is fixed and undeclared.** `input.sample_rate` is the
//! only field, and everything else is assumed: signed 16-bit, little endian,
//! mono, raw - no WAV header, despite the metadata Home Assistant fills in
//! saying `AudioFormats.WAV`. A rate other than 16000 is accepted and
//! resampled server-side with `audioop.ratecv`, so sending the device's native
//! rate is legitimate; sending 16000 is one fewer conversion.
//!
//! **Binary frames are multiplexed by a leading byte.** The connection carries
//! more than this pipeline, so every binary frame starts with the handler id
//! from `run-start`, and the audio follows it. A frame containing only that
//! byte is the end of stream - the server's generator is `while chunk := await
//! queue.get()`, so an empty payload is what stops it. Forgetting it leaves
//! the pipeline waiting until its timeout.
//!
//! **Voice activity detection is on and cannot be turned off for this stage.**
//! `no_vad` exists in the schema for the wake-word start stage only, and the
//! `input` sub-schema rejects unknown keys, so a push-to-talk client still has
//! its stream ended by the server after 0.7s of silence. That is usually what
//! you want; it is not what you asked for, and a long pause mid-sentence ends
//! the utterance. The same schema is why `volume_multiplier` and
//! `auto_gain_dbfs` are unavailable here: a quiet microphone has to be
//! amplified on the device.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::ws::{Message, WebSocket};
use crate::Source;

pub const DEFAULT_PORT: u16 = 8123;
pub const DEFAULT_PATH: &str = "/api/websocket";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Where a pipeline run stops. Speech-to-text alone is the cheap one - it is
/// all a dictation field needs, and it does not ask a language model anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Stt,
    Intent,
    Tts,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Stt => "stt",
            Stage::Intent => "intent",
            Stage::Tts => "tts",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    /// A pipeline id from `pipelines()`. `None` uses the preferred one.
    pub pipeline: Option<String>,
    /// Carry this forward from a previous `Outcome` to keep context between
    /// turns; `None` starts a new conversation.
    pub conversation_id: Option<String>,
    /// The Home Assistant device registry id for this remote, if it has been
    /// registered. It is what lets the assistant resolve "turn off the light"
    /// against the room the remote is in.
    pub device_id: Option<String>,
    pub end_stage: Stage,
    /// How long Home Assistant will let the whole run take.
    pub timeout: Option<f64>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            pipeline: None,
            conversation_id: None,
            device_id: None,
            end_stage: Stage::Intent,
            timeout: None,
        }
    }
}

/// A pipeline event, as it arrives. Everything the API can emit is here;
/// anything unrecognised keeps its name and its payload rather than being
/// dropped, because a new Home Assistant release adding an event should not
/// make this client look broken.
#[derive(Debug, Clone)]
pub enum Event {
    RunStart {
        pipeline: String,
        language: String,
        conversation_id: String,
    },
    SttStart {
        engine: String,
    },
    /// The server heard speech begin, `timestamp` ms into the stream.
    VadStart {
        timestamp: i64,
    },
    /// The server heard the utterance end. The audio stream is over from its
    /// point of view, whether or not the button is still held.
    VadEnd {
        timestamp: i64,
    },
    /// The transcription. This is the one a text field wants.
    SttEnd {
        text: String,
    },
    IntentStart {
        engine: String,
    },
    /// The assistant's spoken answer, plus the whole `intent_output` for a
    /// caller that wants the targets it acted on.
    IntentEnd {
        speech: String,
        output: Value,
    },
    TtsStart {
        text: String,
    },
    /// Where to fetch the spoken reply, relative to the Home Assistant origin.
    TtsEnd {
        url: String,
    },
    RunEnd,
    Error {
        code: String,
        message: String,
    },
    Other {
        kind: String,
        data: Value,
    },
}

/// One configured Assist pipeline. `stt_engine` is the field worth reading:
/// a pipeline with none cannot transcribe anything, which is the most common
/// reason a run fails on a fresh Home Assistant.
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    pub language: String,
    pub stt_engine: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Pipelines {
    pub list: Vec<Pipeline>,
    /// What a run with no `pipeline` will use.
    pub preferred: Option<String>,
}

/// What a run produced. Every field is optional because a run can stop at any
/// stage, or fail before reaching one.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// What the speech-to-text engine heard.
    pub text: Option<String>,
    /// What the assistant said back.
    pub speech: Option<String>,
    pub tts_url: Option<String>,
    pub conversation_id: Option<String>,
    /// Set when the pipeline emitted an error event rather than failing the
    /// command outright.
    pub error: Option<(String, String)>,
}

/// One authenticated connection.
///
/// Not `Sync`, and deliberately: one socket, one read position, and a run that
/// interleaves sending audio with reading events. Two threads sharing this
/// would have one of them stealing the other's replies.
pub struct Assistant {
    ws: WebSocket,
    endpoint: String,
    next_id: u64,
    version: String,
    timeout: Duration,
}

// Derived, this would be a struct with a socket in it and no useful fields.
// Written out, it is the two things a log line wants - and neither of them is
// ever the token, which this struct does not hold in the first place.
impl std::fmt::Debug for Assistant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assistant")
            .field("endpoint", &self.endpoint)
            .field("ha_version", &self.version)
            .finish()
    }
}

impl Assistant {
    /// Connect and authenticate.
    ///
    /// The token is used once, here, and is not stored on the struct - so no
    /// `Debug`, log line or panic message can carry it.
    pub fn connect(host: &str, port: u16, token: &str) -> Result<Assistant> {
        Assistant::connect_with(host, port, DEFAULT_PATH, token, DEFAULT_TIMEOUT)
    }

    pub fn connect_with(
        host: &str,
        port: u16,
        path: &str,
        token: &str,
        timeout: Duration,
    ) -> Result<Assistant> {
        let ws = WebSocket::connect(host, port, path, timeout)?;
        let endpoint = ws.endpoint().to_string();
        let mut ha = Assistant {
            ws,
            endpoint,
            next_id: 1,
            version: String::new(),
            timeout,
        };
        ha.authenticate(token)?;
        Ok(ha)
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The `ha_version` from the greeting. Worth logging: the pipeline API
    /// changed shape before 2023.5 and this client speaks the current one.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The configured pipelines, and which one a run with no `pipeline` will
    /// use.
    pub fn pipelines(&mut self) -> Result<Pipelines> {
        let result = self.command(json!({ "type": "assist_pipeline/pipeline/list" }))?;
        let str_at = |v: &Value, key: &str| {
            v.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let list = result
            .get("pipelines")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|p| Pipeline {
                        id: str_at(p, "id"),
                        name: str_at(p, "name"),
                        language: str_at(p, "language"),
                        stt_engine: p
                            .get("stt_engine")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Pipelines {
            list,
            preferred: result
                .get("preferred_pipeline")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// Run a pipeline from text rather than audio. No microphone involved,
    /// which makes it the way to prove a token, a pipeline and an agent all
    /// work before blaming the hardware.
    pub fn ask(&mut self, text: &str, options: &Options) -> Result<Outcome> {
        let mut request = self.run_request(options, Stage::Intent);
        request["input"] = json!({ "text": text });
        let id = request["id"].as_u64().unwrap_or(0);
        self.send(&request)?;
        self.await_subscription(id)?;
        self.collect(id, &mut |_| {}, None)
    }

    /// Run the audio pipeline: stream `source` and report events as they
    /// arrive. Returns when Home Assistant ends the run.
    ///
    /// `on_event` is called from this thread between chunks, so it must not
    /// block for long - it is what drives a level meter or a partial
    /// transcription on screen.
    pub fn run(
        &mut self,
        source: &mut dyn Source,
        options: &Options,
        on_event: &mut dyn FnMut(&Event),
    ) -> Result<Outcome> {
        let rate = source.rate();
        let mut request = self.run_request(options, Stage::Stt);
        request["input"] = json!({ "sample_rate": rate });
        let id = request["id"].as_u64().unwrap_or(0);
        self.send(&request)?;
        self.await_subscription(id)?;

        // The handler id only exists once run-start has arrived, so the first
        // chunk of audio cannot be sent before it. Capture has already begun -
        // the kernel is buffering - so nothing is lost while we wait.
        let handler = self.await_handler(id, on_event)?;

        let mut samples = vec![0i16; source.chunk_frames().max(1)];
        let mut frame = Vec::with_capacity(1 + samples.len() * 2);
        loop {
            let n = source.read(&mut samples)?;
            if n == 0 {
                break;
            }
            frame.clear();
            frame.push(handler);
            for s in &samples[..n] {
                frame.extend_from_slice(&s.to_le_bytes());
            }
            self.ws.send_binary(&frame)?;

            // Drain whatever came back while that was going out. The server
            // can end the stream early on silence, and continuing to push
            // audio at a finished pipeline is harmless but pointless.
            if self.pump(id, on_event)? {
                break;
            }
        }
        // One bare handler byte: the empty chunk the server's generator stops
        // on. Without it the run sits until its timeout.
        self.ws.send_binary(&[handler])?;

        self.collect(id, on_event, None)
    }

    // --- internals ----------------------------------------------------------

    fn authenticate(&mut self, token: &str) -> Result<()> {
        let greeting = self.expect_message()?;
        match greeting.get("type").and_then(Value::as_str) {
            Some("auth_required") => {}
            Some("auth_ok") => return Ok(()), // an unauthenticated instance
            other => {
                return Err(Error::Ws {
                    endpoint: self.endpoint.clone(),
                    detail: format!(
                        "expected auth_required, got {:?}",
                        other.unwrap_or("no type at all")
                    ),
                })
            }
        }
        self.send(&json!({ "type": "auth", "access_token": token }))?;
        let reply = self.expect_message()?;
        match reply.get("type").and_then(Value::as_str) {
            Some("auth_ok") => {
                self.version = reply
                    .get("ha_version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                Ok(())
            }
            // The message carries a reason, and the reason is about the token.
            // It is not repeated here: an error that quotes what was wrong
            // with a credential is an error that ends up in a log file.
            Some("auth_invalid") => Err(Error::Unauthorized {
                endpoint: self.endpoint.clone(),
            }),
            other => Err(Error::Ws {
                endpoint: self.endpoint.clone(),
                detail: format!("expected auth_ok, got {other:?}"),
            }),
        }
    }

    fn run_request(&mut self, options: &Options, start: Stage) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut request = json!({
            "id": id,
            "type": "assist_pipeline/run",
            "start_stage": start.name(),
            "end_stage": options.end_stage.name(),
        });
        // Absent rather than null: the schema takes `pipeline` as a string, so
        // a null is a validation error rather than "use the preferred one".
        if let Some(p) = &options.pipeline {
            request["pipeline"] = json!(p);
        }
        if let Some(c) = &options.conversation_id {
            request["conversation_id"] = json!(c);
        }
        if let Some(d) = &options.device_id {
            request["device_id"] = json!(d);
        }
        if let Some(t) = options.timeout {
            request["timeout"] = json!(t);
        }
        request
    }

    /// A plain command, with its `result` back.
    fn command(&mut self, mut request: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        request["id"] = json!(id);
        self.send(&request)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let m = self.next_message(deadline)?;
            if m.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            return self.result_of(&m);
        }
    }

    /// `assist_pipeline/run` answers with an empty result to say the
    /// subscription is live; events follow on the same id.
    fn await_subscription(&mut self, id: u64) -> Result<()> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let m = self.next_message(deadline)?;
            if m.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if m.get("type").and_then(Value::as_str) == Some("result") {
                self.result_of(&m)?;
                return Ok(());
            }
        }
    }

    fn await_handler(&mut self, id: u64, on_event: &mut dyn FnMut(&Event)) -> Result<u8> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let m = self.next_message(deadline)?;
            if m.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            let Some(event) = m.get("event") else {
                continue;
            };
            let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
            let data = event.get("data").cloned().unwrap_or(Value::Null);
            on_event(&decode(kind, &data));
            if kind == "error" {
                return Err(self.pipeline_error(&data));
            }
            if kind == "run-start" {
                return data
                    .pointer("/runner_data/stt_binary_handler_id")
                    .and_then(Value::as_u64)
                    .map(|n| n as u8)
                    .ok_or_else(|| Error::Ws {
                        endpoint: self.endpoint.clone(),
                        detail: "run-start carried no stt_binary_handler_id, so there is \
                                 nowhere to send audio - the run did not start at the stt stage"
                            .into(),
                    });
            }
        }
    }

    /// Read whatever is already waiting without blocking, reporting events.
    /// Returns true when the run has ended.
    fn pump(&mut self, id: u64, on_event: &mut dyn FnMut(&Event)) -> Result<bool> {
        // One millisecond, not zero: zero would parse the buffer and never
        // read the socket, so events would only appear once something else
        // happened to fill it.
        while let Some(m) = self.ws.recv(Duration::from_millis(1))? {
            let Message::Text(raw) = m else {
                // Home Assistant sends binary to a client only for things this
                // crate does not subscribe to, and a close is terminal.
                continue;
            };
            let m: Value = serde_json::from_str(&raw)?;
            if m.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(event) = m.get("event") {
                let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
                let data = event.get("data").cloned().unwrap_or(Value::Null);
                on_event(&decode(kind, &data));
                if kind == "run-end" || kind == "error" {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Read events until the run ends, folding them into an `Outcome`.
    fn collect(
        &mut self,
        id: u64,
        on_event: &mut dyn FnMut(&Event),
        budget: Option<Duration>,
    ) -> Result<Outcome> {
        // Generous, because the intent stage can be a language model on the
        // other end of somebody's internet connection. Home Assistant enforces
        // its own timeout and ends the run; this only stops us waiting for a
        // server that has gone away entirely.
        let deadline = Instant::now() + budget.unwrap_or(Duration::from_secs(120));
        let mut outcome = Outcome::default();
        loop {
            let m = self.next_message(deadline)?;
            if m.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if m.get("type").and_then(Value::as_str) == Some("result") {
                self.result_of(&m)?;
                continue;
            }
            let Some(event) = m.get("event") else {
                continue;
            };
            let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
            let data = event.get("data").cloned().unwrap_or(Value::Null);
            let decoded = decode(kind, &data);
            on_event(&decoded);
            match &decoded {
                Event::RunStart {
                    conversation_id, ..
                } => outcome.conversation_id = Some(conversation_id.clone()),
                Event::SttEnd { text } => outcome.text = Some(text.clone()),
                Event::IntentEnd { speech, output } => {
                    outcome.speech = Some(speech.clone());
                    if let Some(c) = output.get("conversation_id").and_then(Value::as_str) {
                        outcome.conversation_id = Some(c.to_string());
                    }
                }
                Event::TtsEnd { url } => outcome.tts_url = Some(url.clone()),
                Event::Error { code, message } => {
                    outcome.error = Some((code.clone(), message.clone()));
                    return Ok(outcome);
                }
                Event::RunEnd => return Ok(outcome),
                _ => {}
            }
        }
    }

    fn pipeline_error(&self, data: &Value) -> Error {
        Error::Ha {
            endpoint: self.endpoint.clone(),
            code: data
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message")
                .to_string(),
        }
    }

    fn result_of(&self, m: &Value) -> Result<Value> {
        if m.get("success").and_then(Value::as_bool) == Some(false) {
            let e = m.get("error").cloned().unwrap_or(Value::Null);
            return Err(Error::Ha {
                endpoint: self.endpoint.clone(),
                code: e
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                message: e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("no message")
                    .to_string(),
            });
        }
        Ok(m.get("result").cloned().unwrap_or(Value::Null))
    }

    fn send(&mut self, value: &Value) -> Result<()> {
        self.ws.send_text(&serde_json::to_string(value)?)
    }

    fn expect_message(&mut self) -> Result<Value> {
        self.next_message(Instant::now() + self.timeout)
    }

    fn next_message(&mut self, deadline: Instant) -> Result<Value> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Io {
                    endpoint: self.endpoint.clone(),
                    source: std::io::Error::new(std::io::ErrorKind::TimedOut, "no reply"),
                });
            }
            match self.ws.recv(left.min(Duration::from_secs(1)))? {
                Some(Message::Text(raw)) => return Ok(serde_json::from_str(&raw)?),
                Some(Message::Binary(_)) => continue,
                Some(Message::Closed) => {
                    return Err(Error::Io {
                        endpoint: self.endpoint.clone(),
                        source: std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "Home Assistant closed the connection",
                        ),
                    })
                }
                None => continue,
            }
        }
    }
}

/// One event object to one `Event`.
fn decode(kind: &str, data: &Value) -> Event {
    let s = |p: &str| -> String {
        data.pointer(p)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match kind {
        "run-start" => Event::RunStart {
            pipeline: s("/pipeline"),
            language: s("/language"),
            conversation_id: s("/conversation_id"),
        },
        "stt-start" => Event::SttStart {
            engine: s("/engine"),
        },
        "stt-vad-start" => Event::VadStart {
            timestamp: data
                .pointer("/timestamp")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        },
        "stt-vad-end" => Event::VadEnd {
            timestamp: data
                .pointer("/timestamp")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        },
        "stt-end" => Event::SttEnd {
            text: s("/stt_output/text"),
        },
        "intent-start" => Event::IntentStart {
            engine: s("/engine"),
        },
        "intent-end" => Event::IntentEnd {
            // The assistant's sentence lives four levels down, and the layers
            // above it are the intent machinery rather than anything a caller
            // wants; `output` carries the lot for those that do.
            speech: s("/intent_output/response/speech/plain/speech"),
            output: data.get("intent_output").cloned().unwrap_or(Value::Null),
        },
        "tts-start" => Event::TtsStart {
            text: s("/tts_input"),
        },
        "tts-end" => Event::TtsEnd {
            url: s("/tts_output/url"),
        },
        "run-end" => Event::RunEnd,
        "error" => Event::Error {
            code: s("/code"),
            message: s("/message"),
        },
        other => Event::Other {
            kind: other.to_string(),
            data: data.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    /// A stand-in for Home Assistant.
    ///
    /// It speaks real WebSocket - unmasked server frames, unmasking the
    /// client's - because the point is to exercise the framing and the
    /// protocol together, and the framing is the half that is impossible to
    /// debug against a real hub over a serial console.
    ///
    /// The script is in phases rather than a flat list, because the real
    /// server does not answer message-for-message: the run command produces a
    /// result and three events, and the rest only arrive once the audio stream
    /// has been terminated. A flat list deadlocks on exactly that.
    struct Script {
        /// Sent as soon as the upgrade completes.
        greeting: Vec<String>,
        /// The nth batch is sent when the nth text frame arrives.
        on_text: Vec<Vec<String>>,
        /// Sent when the client ends the audio stream with a bare handler byte.
        on_end: Vec<String>,
    }

    fn write_text(s: &mut TcpStream, text: &str) {
        let b = text.as_bytes();
        let mut f = vec![0x81u8];
        if b.len() < 126 {
            f.push(b.len() as u8);
        } else {
            f.push(126);
            f.extend_from_slice(&(b.len() as u16).to_be_bytes());
        }
        f.extend_from_slice(b);
        let _ = s.write_all(&f);
    }

    /// Every client frame, unmasked, as (opcode, payload).
    fn take_frames(buf: &mut Vec<u8>) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        while buf.len() >= 2 {
            let opcode = buf[0] & 0x0f;
            let short = (buf[1] & 0x7f) as usize;
            let (len, at) = match short {
                126 if buf.len() >= 4 => (u16::from_be_bytes([buf[2], buf[3]]) as usize, 4),
                127 if buf.len() >= 10 => (
                    u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize,
                    10,
                ),
                126 | 127 => break,
                n => (n, 2),
            };
            // Every client frame is masked; the client is what we are testing,
            // so this asserts rather than tolerates.
            assert_eq!(buf[1] & 0x80, 0x80, "client frames must be masked");
            if buf.len() < at + 4 + len {
                break;
            }
            let mask = [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
            let payload: Vec<u8> = buf[at + 4..at + 4 + len]
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ mask[i & 3])
                .collect();
            buf.drain(..at + 4 + len);
            out.push((opcode, payload));
        }
        out
    }

    fn fake_ha(script: Script) -> (u16, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
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
            let accept = crate::ws::base64(&crate::ws::sha1(
                format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
            ));
            s.write_all(
                format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
                     Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                )
                .as_bytes(),
            )
            .unwrap();
            for line in &script.greeting {
                write_text(&mut s, line);
            }

            let mut audio = Vec::new();
            let mut texts = 0usize;
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&chunk[..n]);
                for (opcode, payload) in take_frames(&mut buf) {
                    match opcode {
                        crate::ws::OP_TEXT => {
                            if let Some(batch) = script.on_text.get(texts) {
                                for line in batch {
                                    write_text(&mut s, line);
                                }
                            }
                            texts += 1;
                        }
                        crate::ws::OP_BINARY => {
                            let end = payload.len() == 1;
                            audio.push(payload);
                            if end {
                                for line in &script.on_end {
                                    write_text(&mut s, line);
                                }
                            }
                        }
                        crate::ws::OP_CLOSE => return audio,
                        _ => {}
                    }
                }
            }
            audio
        });
        (port, handle)
    }

    fn event(id: u64, kind: &str, data: Value) -> String {
        json!({"id": id, "type": "event", "event": {"type": kind, "data": data}}).to_string()
    }

    /// The contract that matters end to end: the transcription comes back, the
    /// assistant's reply comes back, every audio frame carried the handler id,
    /// and the stream was terminated with a bare handler byte - the omission
    /// that leaves a real pipeline hanging until its timeout.
    #[test]
    fn a_whole_pipeline_run_yields_text_and_a_response() {
        let script = Script {
            greeting: vec![json!({"type": "auth_required", "ha_version": "2026.9.0"}).to_string()],
            on_text: vec![
                vec![json!({"type": "auth_ok", "ha_version": "2026.9.0"}).to_string()],
                vec![
                    json!({"id": 1, "type": "result", "success": true, "result": null}).to_string(),
                    event(
                        1,
                        "run-start",
                        json!({"pipeline": "01ABC", "language": "en",
                               "conversation_id": "conv-1",
                               "runner_data": {"stt_binary_handler_id": 7, "timeout": 300}}),
                    ),
                    event(1, "stt-start", json!({"engine": "stt.whisper"})),
                    event(1, "stt-vad-start", json!({"timestamp": 120})),
                ],
            ],
            on_end: vec![
                event(1, "stt-vad-end", json!({"timestamp": 1400})),
                event(
                    1,
                    "stt-end",
                    json!({"stt_output": {"text": "turn on the lamp"}}),
                ),
                event(
                    1,
                    "intent-start",
                    json!({"engine": "conversation.home_assistant"}),
                ),
                event(
                    1,
                    "intent-end",
                    json!({"intent_output": {
                        "response": {"speech": {"plain": {"speech": "Turned on the lamp"}}},
                        "conversation_id": "conv-1"}}),
                ),
                event(1, "run-end", Value::Null),
            ],
        };
        let (port, server) = fake_ha(script);

        let mut ha = Assistant::connect("127.0.0.1", port, "a-token").unwrap();
        assert_eq!(ha.version(), "2026.9.0");

        let samples: Vec<i16> = (0..2000).map(|i| (i % 300) as i16).collect();
        let mut source = crate::wav::Reader::from_samples(samples, 16000).with_chunk(512);

        let mut seen: Vec<String> = Vec::new();
        let outcome = ha
            .run(&mut source, &Options::default(), &mut |e| {
                seen.push(format!("{e:?}"))
            })
            .unwrap();

        assert_eq!(outcome.text.as_deref(), Some("turn on the lamp"));
        assert_eq!(outcome.speech.as_deref(), Some("Turned on the lamp"));
        assert_eq!(outcome.conversation_id.as_deref(), Some("conv-1"));
        assert!(outcome.error.is_none());
        assert!(seen.iter().any(|e| e.starts_with("VadStart")), "{seen:?}");
        assert!(seen.iter().any(|e| e.starts_with("RunEnd")), "{seen:?}");

        drop(ha);
        let audio = server.join().unwrap();
        assert!(audio.len() >= 2, "audio and a terminator");
        assert!(
            audio.iter().all(|f| f[0] == 7),
            "every binary frame carries the handler id"
        );
        assert_eq!(
            audio.last().unwrap().len(),
            1,
            "the stream ends with a bare handler byte"
        );
        let sent: usize = audio.iter().map(|f| f.len() - 1).sum();
        assert_eq!(sent, 2000 * 2, "every sample went out, once");
    }

    #[test]
    fn a_refused_token_is_unauthorized_and_says_nothing_about_it() {
        let (port, server) = fake_ha(Script {
            greeting: vec![json!({"type": "auth_required", "ha_version": "2026.9.0"}).to_string()],
            on_text: vec![vec![json!({"type": "auth_invalid",
                                      "message": "Invalid access token or password"})
            .to_string()]],
            on_end: vec![],
        });
        let e = Assistant::connect("127.0.0.1", port, "sekrit-token").unwrap_err();
        assert!(matches!(e, Error::Unauthorized { .. }));
        // The whole point of not storing the token: nothing that can reach a
        // log file can carry it.
        let shown = format!("{e} {e:?}");
        assert!(
            !shown.contains("sekrit"),
            "the token must not be in {shown}"
        );
        let _ = server.join();
    }

    /// A pipeline with no speech-to-text configured answers with an error
    /// event before run-start, so the handler id never arrives. That has to
    /// end the run rather than wait for the audio it can never address.
    #[test]
    fn a_pipeline_error_event_ends_the_run_rather_than_hanging() {
        let (port, server) = fake_ha(Script {
            greeting: vec![json!({"type": "auth_required"}).to_string()],
            on_text: vec![
                vec![json!({"type": "auth_ok", "ha_version": "2026.9.0"}).to_string()],
                vec![
                    json!({"id": 1, "type": "result", "success": true}).to_string(),
                    event(
                        1,
                        "error",
                        json!({"code": "stt-provider-missing",
                               "message": "No speech-to-text provider"}),
                    ),
                ],
            ],
            on_end: vec![],
        });
        let mut ha = Assistant::connect("127.0.0.1", port, "t").unwrap();
        let mut source = crate::wav::Reader::from_samples(vec![0; 100], 16000).with_chunk(100);
        let e = ha
            .run(&mut source, &Options::default(), &mut |_| {})
            .unwrap_err();
        assert!(e.to_string().contains("No speech-to-text provider"), "{e}");
        drop(ha);
        let _ = server.join();
    }

    /// A run command Home Assistant refuses outright - a pipeline id that does
    /// not exist - comes back as a failed result, not an event.
    #[test]
    fn a_refused_run_command_is_reported_with_its_reason() {
        let (port, server) = fake_ha(Script {
            greeting: vec![json!({"type": "auth_required"}).to_string()],
            on_text: vec![
                vec![json!({"type": "auth_ok", "ha_version": "2026.9.0"}).to_string()],
                vec![json!({"id": 1, "type": "result", "success": false,
                            "error": {"code": "pipeline-not-found",
                                      "message": "Pipeline not found: id=nope"}})
                .to_string()],
            ],
            on_end: vec![],
        });
        let mut ha = Assistant::connect("127.0.0.1", port, "t").unwrap();
        let mut source = crate::wav::Reader::from_samples(vec![0; 100], 16000);
        let options = Options {
            pipeline: Some("nope".into()),
            ..Options::default()
        };
        let e = ha.run(&mut source, &options, &mut |_| {}).unwrap_err();
        assert!(e.to_string().contains("Pipeline not found"), "{e}");
        drop(ha);
        let _ = server.join();
    }

    #[test]
    fn events_decode_including_ones_this_client_has_never_seen() {
        let e = decode("stt-end", &json!({"stt_output": {"text": "hello"}}));
        assert!(matches!(e, Event::SttEnd { text } if text == "hello"));
        let e = decode(
            "intent-progress",
            &json!({"chat_log_delta": {"role": "assistant"}}),
        );
        assert!(matches!(e, Event::Other { kind, .. } if kind == "intent-progress"));
        // A shape missing the field we want must not panic - a new Home
        // Assistant release is allowed to move things.
        let e = decode("stt-end", &Value::Null);
        assert!(matches!(e, Event::SttEnd { text } if text.is_empty()));
        let e = decode(
            "tts-end",
            &json!({"tts_output": {"url": "/api/tts_proxy/x.mp3"}}),
        );
        assert!(matches!(e, Event::TtsEnd { url } if url.ends_with("x.mp3")));
    }

    /// The request has to leave out what it does not have rather than send
    /// nulls: `pipeline` is `str` in the schema, so a null fails validation
    /// instead of meaning "the preferred one".
    #[test]
    fn an_unset_pipeline_is_absent_not_null() {
        let (port, server) = fake_ha(Script {
            greeting: vec![json!({"type": "auth_required"}).to_string()],
            on_text: vec![vec![json!({"type": "auth_ok"}).to_string()]],
            on_end: vec![],
        });
        let mut ha = Assistant::connect("127.0.0.1", port, "t").unwrap();
        let request = ha.run_request(&Options::default(), Stage::Stt);
        assert!(request.get("pipeline").is_none());
        assert!(request.get("conversation_id").is_none());
        assert_eq!(request["start_stage"], "stt");
        assert_eq!(request["end_stage"], "intent");
        drop(ha);
        let _ = server.join();
    }
}
