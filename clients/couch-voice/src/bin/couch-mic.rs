//! The microphone probe: everything about the audio hardware, and nothing
//! about the network.
//!
//! This binary exists separately from `couch-voice` for one reason. It is the
//! thing somebody is asked to run on a device with a microphone in it, and it
//! contains no code that could send audio anywhere - no WebSocket, no HTTP, no
//! sockets at all. What it records goes to a file on the device, or nowhere.
//!
//! It answers the questions the hardware has not answered yet:
//!
//! * which of this card's many capture devices is the microphone
//!   (`couch-mic sweep`);
//! * what formats and rates each accepts (`couch-mic list`);
//! * what the mixer looks like and which controls gate the analogue path
//!   (`couch-mic controls`);
//! * and whether a recording contains a voice, a hiss, or nothing at all
//!   (`couch-mic record`, `couch-mic analyse`).
//!
//! The verdicts are deliberately blunt. "It sounds quiet" is not a result
//! somebody can act on from the other end of a chat window.

use std::process::ExitCode;
use std::time::Duration;

use couch_voice::abi;
use couch_voice::alsa::{self, Pcm, Wanted};
use couch_voice::ctl::{Control, MTK_AMIC_ROUTE};
use couch_voice::level::{analyse, Analysis};
use couch_voice::wav;
use couch_voice::Source;

const USAGE: &str = "\
usage: couch-mic [command] [options]

Probes the capture hardware. Records only when told to, only to a file, and
never over a network - this binary has no network code in it.

commands:
  list                 every capture device, and what each will accept
  sweep                record briefly from every device and say which one hears
  record               record one device to a WAV and analyse it
  analyse FILE.wav     the same analysis, on a file that already exists
  controls [MATCH]     the card's mixer controls, their values and their items
  set NAME=VALUE ...   set mixer controls by name
  route                apply the MediaTek analogue-microphone route (see --dry-run)

options:
  -D, --device SPEC    hw:0,1 / 0,1 / 1 / a /dev/snd path      (default hw:0,1)
  -c, --card N         which card's mixer to talk to           (default 0)
  -r, --rate HZ        capture rate                            (default 16000)
      --channels N     capture channels                        (default 2)
  -t, --seconds S      how long to record                      (default 3, sweep 1.5)
  -o, --out FILE       where to write the WAV             (default /tmp/mic.wav)
      --dry-run        for `route`: print what it would set, change nothing
  -v, --verbose        for `controls`: every control, not just the audio ones
";

macro_rules! say {
    ($($a:tt)*) => {{
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        if writeln!(out, $($a)*).is_err() {
            std::process::exit(0);
        }
    }};
}

struct Args {
    device: String,
    card: u32,
    rate: u32,
    channels: u32,
    seconds: f64,
    seconds_given: bool,
    out: String,
    dry_run: bool,
    verbose: bool,
    words: Vec<String>,
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS,
        Err(m) => {
            eprintln!("couch-mic: {m}\ntry --help");
            return ExitCode::from(2);
        }
    };
    let command = args.words.first().cloned().unwrap_or_else(|| "list".into());
    let result = match command.as_str() {
        "list" => list(&args),
        "sweep" => sweep(&args),
        "record" => record(&args),
        "analyse" | "analyze" => analyse_file(&args),
        "controls" => controls(&args),
        "set" => set(&args),
        "route" => route(&args),
        other => {
            eprintln!("couch-mic: unknown command {other}\ntry --help");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("couch-mic: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse() -> Result<Option<Args>, String> {
    let mut a = Args {
        device: "hw:0,1".into(),
        card: 0,
        rate: alsa::HA_RATE,
        // Two, not one. This codec's uplink is stereo and delivers two
        // channels whatever it agrees to; asking for one yields twice the
        // frames in half the time, which is speech at half speed. See the note
        // on Wanted::default and on demux in alsa.rs.
        channels: 2,
        seconds: 3.0,
        seconds_given: false,
        out: "/tmp/mic.wav".into(),
        dry_run: false,
        verbose: false,
        words: Vec::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| -> Result<String, String> {
            args.next().ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "-D" | "--device" => a.device = value("--device")?,
            "-c" | "--card" => a.card = num(&value("--card")?, "--card")?,
            "-r" | "--rate" => a.rate = num(&value("--rate")?, "--rate")?,
            "--channels" => a.channels = num(&value("--channels")?, "--channels")?,
            "-t" | "--seconds" => {
                a.seconds = num(&value("--seconds")?, "--seconds")?;
                a.seconds_given = true;
            }
            "-o" | "--out" => a.out = value("--out")?,
            "--dry-run" => a.dry_run = true,
            "-v" | "--verbose" => a.verbose = true,
            "-h" | "--help" => {
                say!("{USAGE}");
                return Ok(None);
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                return Err(format!("unknown option {arg}"))
            }
            _ => a.words.push(arg),
        }
    }
    Ok(Some(a))
}

fn num<T: std::str::FromStr>(s: &str, flag: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("{flag} does not take {s:?}"))
}

// --- commands ---------------------------------------------------------------

/// Every capture device, with what HW_REFINE says it will take. Refine changes
/// nothing and does not need the device to be startable, so this is safe to
/// run against a card in any state.
fn list(args: &Args) -> couch_voice::Result<()> {
    card_header(args.card);
    let devices = alsa::capture_devices().unwrap_or_default();
    if devices.is_empty() {
        say!("no capture devices under /dev/snd");
        say!("(there is no devtmpfs on this kernel, so check `mdev -s` has run)");
        return Ok(());
    }
    for (card, device) in devices {
        let spec = format!("hw:{card},{device}");
        say!("");
        match Pcm::open(&spec) {
            Err(e) => say!("{spec:<10} unavailable: {e}"),
            Ok(pcm) => {
                let info = pcm.info()?;
                say!(
                    "{spec:<10} id={:?} name={:?}{}",
                    info.id,
                    info.name,
                    if info.subdevices > 1 {
                        format!(" subdevices={}", info.subdevices)
                    } else {
                        String::new()
                    }
                );
                match pcm.caps() {
                    Err(e) => say!("           caps unavailable: {e}"),
                    Ok(caps) => {
                        say!(
                            "           access   {}",
                            caps.access
                                .iter()
                                .map(|a| abi::access_name(*a))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        say!(
                            "           formats  {}",
                            caps.formats
                                .iter()
                                .map(|f| abi::format_name(*f))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        say!(
                            "           channels {}..{}   rates {}..{}   period {}..{}",
                            caps.channels.0,
                            caps.channels.1,
                            caps.rates.0,
                            caps.rates.1,
                            caps.period_frames.0,
                            caps.period_frames.1
                        );
                    }
                }
                let wanted = pcm.accepts(args.rate, args.channels, abi::FORMAT_S16_LE)?;
                say!(
                    "           {} Hz / {} ch / S16_LE: {}",
                    args.rate,
                    args.channels,
                    if wanted { "YES" } else { "no" }
                );
            }
        }
    }
    Ok(())
}

/// The one that answers "which device is the microphone". Records briefly from
/// every capture device that will take the wanted format and prints one line
/// per device, ending in a verdict.
fn sweep(args: &Args) -> couch_voice::Result<()> {
    card_header(args.card);
    let seconds = if args.seconds_given {
        args.seconds
    } else {
        1.5
    };
    say!("");
    say!(
        "recording {seconds:.1}s from each capture device that accepts {} Hz mono S16_LE.",
        args.rate
    );
    say!("Make a noise - talk, tap the case - for the whole sweep.");
    say!("");
    say!(
        "{:<11} {:<28} {:<9} {}",
        "device",
        "pcm id",
        "verdict",
        "peak/rms dBFS  envelope"
    );

    let devices = alsa::capture_devices().unwrap_or_default();
    let mut heard: Vec<(String, String, f32)> = Vec::new();
    for (card, device) in devices {
        let spec = format!("hw:{card},{device}");
        let pcm = match Pcm::open(&spec) {
            Ok(p) => p,
            Err(e) => {
                say!(
                    "{spec:<11} {:<28} {:<9} {}",
                    "-",
                    "unavailable",
                    short(&e.to_string())
                );
                continue;
            }
        };
        let id = pcm.info().map(|i| i.id).unwrap_or_default();
        match pcm.accepts(args.rate, args.channels, abi::FORMAT_S16_LE) {
            Ok(false) => {
                say!("{spec:<11} {id:<28} {:<9} -", "skipped");
                continue;
            }
            Err(e) => {
                say!(
                    "{spec:<11} {id:<28} {:<9} {}",
                    "error",
                    short(&e.to_string())
                );
                continue;
            }
            Ok(true) => {}
        }
        match capture(&spec, args, seconds) {
            Err(e) => say!(
                "{spec:<11} {id:<28} {:<9} {}",
                "error",
                short(&e.to_string())
            ),
            Ok((samples, _)) => {
                let a = analyse(&samples, args.rate);
                say!(
                    "{spec:<11} {id:<28} {:<9} {:>6.1} / {:>6.1}   {}",
                    format!("{:?}", a.verdict).to_uppercase(),
                    a.peak_dbfs(),
                    a.rms_dbfs(),
                    a.sparkline()
                );
                heard.push((spec, id, a.peak_dbfs()));
            }
        }
    }

    say!("");
    let best = heard
        .iter()
        .filter(|(_, _, peak)| peak.is_finite())
        .max_by(|a, b| a.2.total_cmp(&b.2));
    match best {
        Some((spec, id, peak)) if *peak > -60.0 => {
            say!("loudest: {spec} ({id}) at {peak:.1} dBFS peak.");
            say!("Record it properly:  couch-mic record -D {spec} -t 5 -o /tmp/mic.wav");
        }
        _ => {
            say!("Nothing heard anything. Before concluding the microphone is not wired:");
            say!("  couch-mic controls          - is there an ADC/preamp/mic-source control?");
            say!("  couch-mic route --dry-run   - the route Android's HAL uses");
            say!("  couch-mic route             - apply it, then sweep again");
        }
    }
    Ok(())
}

fn record(args: &Args) -> couch_voice::Result<()> {
    say!(
        "recording {:.1}s from {} at {} Hz, {} channel(s) -> {}",
        args.seconds,
        args.device,
        args.rate,
        args.channels,
        args.out
    );
    let (samples, overruns) = capture(&args.device, args, args.seconds)?;

    let mut w = wav::Writer::create(&args.out, args.rate, 1)?;
    w.write(&samples)?;
    w.finish()?;

    let a = analyse(&samples, args.rate);
    report(&a, &args.out);
    if overruns > 0 {
        say!("");
        say!("{overruns} overrun(s): the kernel dropped audio while we were elsewhere.");
        say!("Harmless once or twice; a lot of them means the period is too small.");
    }
    Ok(())
}

fn analyse_file(args: &Args) -> couch_voice::Result<()> {
    let path = args
        .words
        .get(1)
        .cloned()
        .unwrap_or_else(|| args.out.clone());
    let r = wav::Reader::open(&path)?;
    let a = analyse(&r.samples, r.rate);
    report(&a, &path);
    Ok(())
}

fn controls(args: &Args) -> couch_voice::Result<()> {
    let ctl = Control::open(args.card)?;
    let card = ctl.card()?;
    let (major, minor, sub) = ctl.protocol()?;
    say!("card {}: {} \"{}\"", card.index, card.id, card.name);
    say!("  driver     {}", card.driver);
    say!("  longname   {}", card.longname);
    say!("  mixer      {}", card.mixername);
    say!("  components {}", card.components);
    say!("  control    {} protocol {major}.{minor}.{sub}", ctl.path());

    let filter = args.words.get(1).map(|s| s.to_ascii_lowercase());
    let elements = ctl.elements()?;
    say!("");
    say!("{} control(s):", elements.len());
    say!("{:>4}  {:<34} {:<11} {}", "id", "name", "type", "value");
    let mut shown = 0;
    for e in &elements {
        let name = e.name.to_ascii_lowercase();
        let interesting = args.verbose
            || match &filter {
                Some(f) => name.contains(f.as_str()),
                // The capture path, which is what this probe is for. Anything
                // else on a phone codec is playback, FM radio or the modem.
                None => [
                    "adc", "mic", "preamp", "pga", "capture", "clk_buf", "vow", "loopback",
                ]
                .iter()
                .any(|k| name.contains(k)),
            };
        if interesting {
            say!("{}", e.line());
            shown += 1;
        }
    }
    if shown == 0 {
        say!("(nothing matched; -v shows all of them)");
    } else if filter.is_none() && !args.verbose {
        say!("");
        say!(
            "(capture-path controls only; -v for all {}, or pass a substring)",
            elements.len()
        );
    }
    Ok(())
}

fn set(args: &Args) -> couch_voice::Result<()> {
    let ctl = Control::open(args.card)?;
    let mut any = false;
    for pair in args.words.iter().skip(1) {
        let Some((name, value)) = pair.split_once('=') else {
            eprintln!("couch-mic: {pair:?} is not NAME=VALUE");
            continue;
        };
        any = true;
        match ctl.set(name, 0, value) {
            Ok(e) => say!("{}", e.line()),
            Err(e) => say!("{name}: {e}"),
        }
    }
    if !any {
        say!("nothing to set. Try: couch-mic set Audio_ADC_1_Switch=on");
    }
    Ok(())
}

/// Apply the route Android's audio HAL uses for this board's analogue
/// microphone. Every step prints what the control ended up at, because the
/// codec clamps some of them and a route that silently did not take is the
/// thing this whole probe exists to expose.
fn route(args: &Args) -> couch_voice::Result<()> {
    let ctl = Control::open(args.card)?;
    if args.dry_run {
        say!("would set, in order:");
        for (name, value) in MTK_AMIC_ROUTE {
            match ctl.get(name, 0) {
                Ok(e) => say!("  {name}={value}   now: {}", e.line().trim()),
                Err(e) => say!("  {name}={value}   (not on this card: {e})"),
            }
        }
        return Ok(());
    }
    for (name, value) in MTK_AMIC_ROUTE {
        match ctl.set(name, 0, value) {
            Ok(e) => say!("ok    {}", e.line()),
            Err(e) => say!("FAIL  {name}={value}: {e}"),
        }
    }
    say!("");
    say!("Now: couch-mic record -D {} -t 5", args.device);
    Ok(())
}

// --- shared -----------------------------------------------------------------

fn card_header(card: u32) {
    match Control::open(card) {
        Ok(ctl) => match ctl.card() {
            Ok(c) => say!("card {}: {} \"{}\" ({})", c.index, c.id, c.name, c.driver),
            Err(e) => say!("card {card}: {e}"),
        },
        Err(e) => say!("card {card}: {e}"),
    }
}

fn capture(spec: &str, args: &Args, seconds: f64) -> couch_voice::Result<(Vec<i16>, u32)> {
    let wanted = Wanted {
        rate: args.rate,
        channels: args.channels,
        sample_format: abi::FORMAT_S16_LE,
        limit: Duration::from_secs_f64(seconds.clamp(0.1, 120.0)),
        ..Wanted::default()
    };
    let mut capture = Pcm::open(spec)?.configure(&wanted)?;
    let mut buf = vec![0i16; capture.chunk_frames().max(1)];
    let mut samples = Vec::with_capacity((args.rate as f64 * seconds) as usize);
    loop {
        let n = capture.read(&mut buf)?;
        if n == 0 {
            break;
        }
        samples.extend_from_slice(&buf[..n]);
    }
    Ok((samples, capture.overruns()))
}

fn report(a: &Analysis, path: &str) {
    say!("");
    say!(
        "{path}: {:.2}s, {} frames at {} Hz",
        a.seconds(),
        a.frames,
        a.rate
    );
    say!("  peak       {:.1} dBFS", a.peak_dbfs());
    say!("  rms        {:.1} dBFS", a.rms_dbfs());
    say!("  dc offset  {:+.4} of full scale", a.dc_offset);
    say!("  range      {} .. {}", a.min, a.max);
    say!(
        "  distinct   {}{}",
        a.distinct,
        if a.distinct >= 4096 {
            "+ (counted to a cap)"
        } else {
            ""
        }
    );
    say!("  zeros      {} of {}", a.zeros, a.frames);
    if a.clipped > 0 {
        say!("  clipped    {} samples", a.clipped);
    }
    if !a.windows.is_empty() {
        say!(
            "  envelope   {}   (one character per 100ms, quiet to loud)",
            a.sparkline()
        );
    }
    say!("");
    say!("  {}", a.verdict.line());
}

/// One line of an error, for a table.
fn short(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.len() > 58 {
        format!("{}...", &line[..55])
    } else {
        line.to_string()
    }
}
