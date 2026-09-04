//! A one-shot CLI over the client, for exercising a hub and a microphone from
//! a laptop or from the device before any daemon exists. Arguments are parsed
//! by hand: a dozen flags do not repay a dependency, least of all on a target
//! we cross-compile.
//!
//! Every command that records says so before it starts and reports what it
//! sent afterwards. That is not decoration - a microphone that can be opened
//! without the person in the room knowing is the thing this whole design is
//! trying not to build.

use std::process::ExitCode;
use std::time::Duration;

use couch_voice::abi;
use couch_voice::alsa::{self, Pcm, Wanted};
use couch_voice::ha::{self, Assistant, Event, Options, Stage};
use couch_voice::level::{analyse, dbfs};
use couch_voice::wav;
use couch_voice::Source;

const USAGE: &str = "\
usage: couch-voice [options] <command>

Records from the remote's microphone and runs it through Home Assistant's
Assist pipeline, or does either half on its own.

commands:
  version              connect, and say what is on the other end
  pipelines            the configured Assist pipelines, and the preferred one
  say TEXT...          run the pipeline from text; no microphone involved
  send FILE.wav        stream a recording to the pipeline; no microphone either
  listen               record from the microphone and stream it
  record FILE.wav      record to a file and stop; Home Assistant is not contacted

options:
  -H, --host HOST      Home Assistant                         (required but for record)
  -p, --port PORT      its port                                        (default 8123)
      --path PATH      the WebSocket endpoint             (default /api/websocket)
      --token-file F   a file containing the long-lived access token
      --pipeline ID    which pipeline; the preferred one by default
      --stage STAGE    where to stop: stt, intent or tts             (default intent)
      --conversation C carry a conversation id forward between turns
      --device-id ID   this remote's Home Assistant device registry id
  -D, --device SPEC    capture device: hw:0,1 / 0,1 / 1 / a path   (default hw:0,1)
  -r, --rate HZ        capture rate; anything else is resampled     (default 16000)
      --channels N     capture channels                                 (default 2)
  -t, --seconds S      how long to record before stopping             (default 10)
      --json           print the events as they arrive, one per line

The token is read from --token-file, then COUCH_HA_TOKEN, then
/opt/couch/ha-token. There is deliberately no --token flag: a command line is
visible to anything that can run ps.";

/// Where the token lives on the device. Beside the config the daemon owns,
/// on the Alpine root rather than in /tmp, and expected to be 0600.
const TOKEN_FILE: &str = "/opt/couch/ha-token";

macro_rules! say {
    ($($a:tt)*) => {{
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        if writeln!(out, $($a)*).is_err() {
            std::process::exit(0);
        }
    }};
}

enum Fail {
    Usage(String),
    /// Already reported; carry the exit status out.
    Silent,
    Voice(couch_voice::Error),
}

impl From<couch_voice::Error> for Fail {
    fn from(e: couch_voice::Error) -> Self {
        Fail::Voice(e)
    }
}

struct Args {
    host: Option<String>,
    port: u16,
    path: String,
    token_file: Option<String>,
    pipeline: Option<String>,
    stage: Stage,
    conversation: Option<String>,
    device_id: Option<String>,
    device: String,
    rate: u32,
    channels: u32,
    seconds: f64,
    json: bool,
    words: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail::Usage(m)) => {
            eprintln!("couch-voice: {m}\ntry --help");
            ExitCode::from(2)
        }
        Err(Fail::Silent) => ExitCode::from(2),
        Err(Fail::Voice(e)) => {
            eprintln!("couch-voice: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Fail> {
    let args = parse()?;
    let Some(command) = args.words.first().cloned() else {
        eprintln!("{USAGE}");
        return Err(Fail::Silent);
    };
    match command.as_str() {
        "version" => version(&args),
        "pipelines" => pipelines(&args),
        "say" => ask(&args),
        "send" => send(&args),
        "listen" => listen(&args),
        "record" => record(&args),
        other => Err(Fail::Usage(format!("unknown command {other}"))),
    }
}

fn parse() -> Result<Args, Fail> {
    let mut a = Args {
        host: None,
        port: ha::DEFAULT_PORT,
        path: ha::DEFAULT_PATH.to_string(),
        token_file: None,
        pipeline: None,
        stage: Stage::Intent,
        conversation: None,
        device_id: None,
        device: "hw:0,1".into(),
        rate: alsa::HA_RATE,
        // Two, not one. This codec's uplink is stereo and delivers two
        // channels whatever it agrees to; asking for one yields twice the
        // frames in half the time, which is speech at half speed. See the note
        // on Wanted::default and on demux in alsa.rs.
        channels: 2,
        seconds: 10.0,
        json: false,
        words: Vec::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| -> Result<String, Fail> {
            args.next()
                .ok_or_else(|| Fail::Usage(format!("{flag} needs a value")))
        };
        match arg.as_str() {
            "-H" | "--host" => a.host = Some(value("--host")?),
            "-p" | "--port" => a.port = num(&value("--port")?, "--port")?,
            "--path" => a.path = value("--path")?,
            "--token-file" => a.token_file = Some(value("--token-file")?),
            "--pipeline" => a.pipeline = Some(value("--pipeline")?),
            "--conversation" => a.conversation = Some(value("--conversation")?),
            "--device-id" => a.device_id = Some(value("--device-id")?),
            "--stage" => {
                a.stage = match value("--stage")?.as_str() {
                    "stt" => Stage::Stt,
                    "intent" => Stage::Intent,
                    "tts" => Stage::Tts,
                    other => {
                        return Err(Fail::Usage(format!(
                            "--stage takes stt, intent or tts, not {other:?}"
                        )))
                    }
                }
            }
            "-D" | "--device" => a.device = value("--device")?,
            "-r" | "--rate" => a.rate = num(&value("--rate")?, "--rate")?,
            "--channels" => a.channels = num(&value("--channels")?, "--channels")?,
            "-t" | "--seconds" => a.seconds = num(&value("--seconds")?, "--seconds")?,
            "--json" => a.json = true,
            "--token" => {
                return Err(Fail::Usage(
                    "there is no --token: a command line is visible in ps. \
                     Use --token-file, or COUCH_HA_TOKEN."
                        .into(),
                ))
            }
            "-h" | "--help" => {
                say!("{USAGE}");
                std::process::exit(0);
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                return Err(Fail::Usage(format!("unknown option {arg}")))
            }
            _ => a.words.push(arg),
        }
    }
    Ok(a)
}

fn num<T: std::str::FromStr>(s: &str, flag: &str) -> Result<T, Fail> {
    s.parse()
        .map_err(|_| Fail::Usage(format!("{flag} does not take {s:?}")))
}

// --- commands ---------------------------------------------------------------

fn version(args: &Args) -> Result<(), Fail> {
    let mut ha = connect(args)?;
    say!("{} is Home Assistant {}", ha.endpoint(), ha.version());
    let pipelines = ha.pipelines()?;
    say!("{} pipeline(s) configured", pipelines.list.len());
    if let Some(p) = pipelines.preferred {
        say!("preferred: {p}");
    }
    Ok(())
}

fn pipelines(args: &Args) -> Result<(), Fail> {
    let mut ha = connect(args)?;
    let pipelines = ha.pipelines()?;
    for p in &pipelines.list {
        let star = if Some(&p.id) == pipelines.preferred.as_ref() {
            "*"
        } else {
            " "
        };
        // A pipeline with no speech-to-text engine will refuse an audio run,
        // and saying so here saves a confusing failure later.
        say!(
            "{star} {}  {:<24} {:<6} stt={}",
            p.id,
            p.name,
            p.language,
            p.stt_engine
                .as_deref()
                .unwrap_or("(none - audio will fail)")
        );
    }
    Ok(())
}

fn ask(args: &Args) -> Result<(), Fail> {
    let text = args.words[1..].join(" ");
    if text.is_empty() {
        return Err(Fail::Usage("say needs something to say".into()));
    }
    let mut ha = connect(args)?;
    let outcome = ha.ask(&text, &options(args))?;
    report(&outcome);
    Ok(())
}

fn send(args: &Args) -> Result<(), Fail> {
    let path = args
        .words
        .get(1)
        .ok_or_else(|| Fail::Usage("send needs a WAV file".into()))?;
    let reader = wav::Reader::open(path)?;
    say!(
        "{path}: {:.2}s at {} Hz -> {}",
        reader.seconds(),
        reader.rate,
        endpoint(args)
    );
    let mut source = reader;
    let mut ha = connect(args)?;
    let outcome = ha.run(&mut source, &options(args), &mut printer(args))?;
    report(&outcome);
    Ok(())
}

fn listen(args: &Args) -> Result<(), Fail> {
    // Connect before opening the microphone. A recording that cannot be sent
    // anywhere is a recording that should not have been made.
    let mut ha = connect(args)?;
    let mut capture = open_capture(args)?;
    eprintln!(
        "recording from {} for up to {:.0}s - speak now (ctrl-c stops)",
        args.device, args.seconds
    );

    // The CLI simply runs to the limit. A UI would hold `capture.stop_handle()`
    // and trip it when the button comes up; there is no button here.
    let mut on_event = printer(args);
    let outcome = ha.run(&mut capture, &options(args), &mut on_event)?;
    eprintln!();
    report(&outcome);
    if capture.overruns() > 0 {
        eprintln!("({} overrun(s) while recording)", capture.overruns());
    }
    Ok(())
}

fn record(args: &Args) -> Result<(), Fail> {
    let path = args
        .words
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/tmp/mic.wav".into());
    let mut capture = open_capture(args)?;
    let format = capture.format();
    eprintln!(
        "recording {:.0}s from {} at {} Hz -> {path}",
        args.seconds, args.device, format.rate
    );

    let mut w = wav::Writer::create(&path, format.rate, 1)?;
    let mut buf = vec![0i16; capture.chunk_frames().max(1)];
    let mut all = Vec::new();
    loop {
        let n = capture.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write(&buf[..n])?;
        all.extend_from_slice(&buf[..n]);
        meter(capture.level().peak);
    }
    w.finish()?;
    eprintln!();

    let a = analyse(&all, format.rate);
    say!(
        "{path}: {:.2}s, peak {:.1} dBFS, rms {:.1} dBFS",
        a.seconds(),
        a.peak_dbfs(),
        a.rms_dbfs()
    );
    say!("{}", a.verdict.line());
    Ok(())
}

// --- shared -----------------------------------------------------------------

fn endpoint(args: &Args) -> String {
    format!(
        "ws://{}:{}{}",
        args.host.as_deref().unwrap_or("?"),
        args.port,
        args.path
    )
}

fn connect(args: &Args) -> Result<Assistant, Fail> {
    let host = args
        .host
        .clone()
        .ok_or_else(|| Fail::Usage("no --host given".into()))?;
    let token = token(args)?;
    Ok(Assistant::connect_with(
        &host,
        args.port,
        &args.path,
        &token,
        ha::DEFAULT_TIMEOUT,
    )?)
}

/// The token, from the least surprising source that has one.
///
/// A file beats the environment because the environment of a process is
/// readable from `/proc` by anything running as the same user, and on this
/// device everything runs as root. Neither is printed, ever.
fn token(args: &Args) -> Result<String, Fail> {
    if let Some(path) = &args.token_file {
        return read_token(path);
    }
    if let Ok(t) = std::env::var("COUCH_HA_TOKEN") {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    if std::path::Path::new(TOKEN_FILE).exists() {
        return read_token(TOKEN_FILE);
    }
    Err(Fail::Usage(format!(
        "no access token. Put a long-lived token in {TOKEN_FILE} (chmod 600), \
         or pass --token-file, or set COUCH_HA_TOKEN"
    )))
}

fn read_token(path: &str) -> Result<String, Fail> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Fail::Usage(format!("cannot read {path}: {e}")))?;
    let token = raw.trim().to_string();
    if token.is_empty() {
        return Err(Fail::Usage(format!("{path} is empty")));
    }
    // Not fatal, because refusing to work would push someone towards putting
    // the token on a command line instead, which is worse.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o077;
            if mode != 0 {
                eprintln!(
                    "couch-voice: warning: {path} is readable by others (mode {:o}); chmod 600 it",
                    meta.permissions().mode() & 0o777
                );
            }
        }
    }
    Ok(token)
}

fn options(args: &Args) -> Options {
    Options {
        pipeline: args.pipeline.clone(),
        conversation_id: args.conversation.clone(),
        device_id: args.device_id.clone(),
        end_stage: args.stage,
        timeout: None,
    }
}

fn open_capture(args: &Args) -> Result<alsa::Capture, Fail> {
    let wanted = Wanted {
        rate: args.rate,
        channels: args.channels,
        sample_format: abi::FORMAT_S16_LE,
        limit: Duration::from_secs_f64(args.seconds.clamp(0.1, 300.0)),
        ..Wanted::default()
    };
    Ok(Pcm::open(&args.device)?.configure(&wanted)?)
}

/// Events go to stderr as they happen, so that stdout carries only the result
/// and can be piped into something.
fn printer(args: &Args) -> impl FnMut(&Event) + '_ {
    let json = args.json;
    move |event: &Event| {
        if json {
            eprintln!("{event:?}");
            return;
        }
        match event {
            Event::RunStart {
                pipeline, language, ..
            } => {
                eprintln!("  pipeline {pipeline} ({language})")
            }
            Event::SttStart { engine } => eprintln!("  listening via {engine}"),
            Event::VadStart { .. } => eprintln!("  speech detected"),
            Event::VadEnd { .. } => eprintln!("  end of speech"),
            Event::SttEnd { text } => eprintln!("  heard: {text:?}"),
            Event::IntentStart { engine } => eprintln!("  asking {engine}"),
            Event::IntentEnd { speech, .. } => eprintln!("  answer: {speech:?}"),
            Event::TtsStart { .. } => {}
            Event::TtsEnd { url } => eprintln!("  speech at {url}"),
            Event::RunEnd => {}
            Event::Error { code, message } => eprintln!("  error [{code}] {message}"),
            Event::Other { kind, .. } => eprintln!("  {kind}"),
        }
    }
}

fn report(outcome: &ha::Outcome) {
    if let Some(text) = &outcome.text {
        say!("heard    {text}");
    }
    if let Some(speech) = &outcome.speech {
        say!("answer   {speech}");
    }
    if let Some(url) = &outcome.tts_url {
        say!("audio    {url}");
    }
    if let Some(id) = &outcome.conversation_id {
        say!("conv     {id}");
    }
    if let Some((code, message)) = &outcome.error {
        say!("error    [{code}] {message}");
    }
}

/// A meter on one line, redrawn in place. On stderr, because it is not output.
fn meter(peak: f32) {
    use std::io::Write as _;
    let db = dbfs(peak);
    let filled = (((db + 60.0) / 60.0).clamp(0.0, 1.0) * 30.0) as usize;
    let bar: String = "#".repeat(filled) + &"-".repeat(30 - filled);
    let _ = write!(std::io::stderr(), "\r  [{bar}] {db:>6.1} dBFS");
    let _ = std::io::stderr().flush();
}
