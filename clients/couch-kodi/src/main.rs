//! A one-shot CLI over the client, for exercising a box from a laptop before
//! the daemon exists. Arguments are parsed by hand: a dozen flags do not repay
//! a dependency, least of all on a target we cross-compile.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use couch_kodi::{Kodi, DEFAULT_HTTP_PORT, DEFAULT_TCP_PORT};

const USAGE: &str = "\
usage: couch-kodi --host H [--port P] [--http [--user U --pass P]]
                 [--web-port P] [--timeout S] <command>

Talks to Kodi's raw JSON-RPC port (9090) by default. --http switches to the web
interface (8080), which has to be enabled in Kodi and cannot receive events.

commands:
  ping                does the box answer
  status              what is playing, and the volume
  up down left right  navigation
  select back home    navigation
  menu                context menu
  playpause stop      transport
  next prev           transport
  seek PERCENT        jump to a point in the item
  volume [LEVEL]      show, or set, 0-100
  mute                toggle
  watch [SECONDS]     print events Kodi pushes; TCP only, 30s by default

The password is also read from COUCH_KODI_PASS, which keeps it off a command
line that anyone with a shell can see in ps.";

/// say! panics when stdout has gone away - a `| head` is enough to do it -
/// and aborting is a poor way to report that nobody is reading any more.
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
    /// Already reported; just carry the exit status out.
    Silent,
    Kodi(couch_kodi::Error),
}

impl From<couch_kodi::Error> for Fail {
    fn from(e: couch_kodi::Error) -> Self {
        Fail::Kodi(e)
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail::Usage(m)) => {
            eprintln!("couch-kodi: {m}\ntry --help");
            ExitCode::from(2)
        }
        Err(Fail::Silent) => ExitCode::from(2),
        Err(Fail::Kodi(e)) => {
            eprintln!("couch-kodi: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Fail> {
    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut web_port: Option<u16> = None;
    let mut over_http = false;
    let mut user: Option<String> = None;
    let mut pass = std::env::var("COUCH_KODI_PASS").ok();
    let mut timeout = 4.0_f64;
    let mut words: Vec<String> = Vec::new();

    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_none() {
        eprintln!("{USAGE}");
        return Err(Fail::Silent);
    }
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| -> Result<String, Fail> {
            args.next()
                .ok_or_else(|| Fail::Usage(format!("{flag} needs a value")))
        };
        match arg.as_str() {
            "--host" | "-H" => host = Some(value("--host")?),
            "--port" | "-p" => port = Some(parse(&value("--port")?, "--port")?),
            "--web-port" => web_port = Some(parse(&value("--web-port")?, "--web-port")?),
            "--http" => over_http = true,
            "--tcp" => over_http = false,
            "--user" | "-u" => user = Some(value("--user")?),
            "--pass" | "-P" => pass = Some(value("--pass")?),
            "--timeout" | "-t" => timeout = parse(&value("--timeout")?, "--timeout")?,
            "--help" | "-h" => {
                say!("{USAGE}");
                return Ok(());
            }
            _ if arg.starts_with('-') => return Err(Fail::Usage(format!("unknown option {arg}"))),
            _ => words.push(arg),
        }
    }

    let host = host.ok_or_else(|| Fail::Usage("no --host given".into()))?;
    let (command, rest) = words
        .split_first()
        .ok_or_else(|| Fail::Usage("no command given".into()))?;

    let port = port.unwrap_or(if over_http {
        DEFAULT_HTTP_PORT
    } else {
        DEFAULT_TCP_PORT
    });
    let mut kodi = if over_http {
        Kodi::http(&host, port)
    } else {
        Kodi::tcp(&host, port)
    }
    .with_timeout(Duration::from_secs_f64(timeout.max(0.1)));
    if let Some(port) = web_port {
        kodi = kodi.with_web_port(port);
    }
    if let Some(user) = &user {
        kodi = kodi.with_auth(user, pass.as_deref().unwrap_or(""));
    }

    match command.as_str() {
        "ping" => {
            kodi.ping()?;
            say!("pong from {}", kodi.endpoint());
        }
        "status" => status(&kodi)?,

        "up" | "down" | "left" | "right" | "select" | "back" | "home" | "menu" => {
            // Kodi's method names are capitalised and the odd one out is the
            // context menu, which nobody would type as "contextmenu".
            let action = match command.as_str() {
                "menu" => "ContextMenu".to_string(),
                other => {
                    let mut s = other.to_string();
                    s[..1].make_ascii_uppercase();
                    s
                }
            };
            kodi.input(&action)?;
        }

        "playpause" | "play" | "pause" => match kodi.play_pause()? {
            Some(true) => say!("paused"),
            Some(false) => say!("playing"),
            None => say!("nothing playing"),
        },
        "stop" => acted(kodi.stop()?, "stopped"),
        "next" => acted(kodi.next()?, "next"),
        "prev" | "previous" => acted(kodi.previous()?, "previous"),
        "seek" => {
            let pct: f64 = parse(
                rest.first()
                    .ok_or_else(|| Fail::Usage("seek needs a percentage".into()))?,
                "seek",
            )?;
            acted(kodi.seek_percent(pct)?, &format!("seeked to {pct:.0}%"));
        }

        "volume" => match rest.first() {
            Some(level) => say!("volume {}", kodi.set_volume(parse(level, "volume")?)?),
            None => {
                let v = kodi.volume()?;
                say!(
                    "volume {}{}",
                    v.volume,
                    if v.muted { " (muted)" } else { "" }
                );
            }
        },
        "mute" => say!(
            "{}",
            if kodi.toggle_mute()? {
                "muted"
            } else {
                "unmuted"
            }
        ),

        "watch" => watch(&kodi, rest.first().map(|s| parse(s, "watch")).transpose()?)?,

        other => return Err(Fail::Usage(format!("unknown command {other}"))),
    }
    Ok(())
}

/// Print what Kodi pushes until the budget runs out. This is the shape the
/// daemon will use: one thread, one blocking wait, no polling.
fn watch(kodi: &Kodi, seconds: Option<f64>) -> Result<(), Fail> {
    if !kodi.supports_notifications() {
        return Err(Fail::Usage(
            "watch needs the TCP transport; the web interface cannot push".into(),
        ));
    }
    let deadline = Instant::now() + Duration::from_secs_f64(seconds.unwrap_or(30.0));
    eprintln!("watching {} - press ctrl-c to stop", kodi.endpoint());
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match kodi.next_notification(left.min(Duration::from_secs(1)))? {
            Some(n) => say!("{}", n.raw),
            None => continue,
        }
    }
    Ok(())
}

fn acted(did: bool, what: &str) {
    say!("{}", if did { what } else { "nothing playing" });
}

fn parse<T: std::str::FromStr>(s: &str, flag: &str) -> Result<T, Fail> {
    s.parse()
        .map_err(|_| Fail::Usage(format!("{flag} does not take {s:?}")))
}

fn status(kodi: &Kodi) -> Result<(), Fail> {
    match kodi.now_playing()? {
        None => say!("nothing playing"),
        Some(np) => {
            say!("{}", np.title);

            let mut sub: Vec<String> = Vec::new();
            if let Some(show) = &np.show {
                sub.push(show.clone());
            }
            if let (Some(s), Some(e)) = (np.season, np.episode) {
                sub.push(format!("S{s:02}E{e:02}"));
            }
            if let Some(y) = np.year {
                sub.push(y.to_string());
            }
            if !sub.is_empty() {
                say!("  {}", sub.join("  "));
            }

            say!(
                "  {}  {} / {}  ({:.1}%)",
                if np.paused { "paused" } else { "playing" },
                hms(np.position_secs),
                hms(np.total_secs),
                np.percentage,
            );

            // The GUI will load these; printing them is the only way to check
            // the encoding against a real library without one.
            for name in ["clearlogo", "fanart", "poster", "thumb"] {
                if let Some(url) = kodi.art_url(&np, name) {
                    say!("  {name:<9} {url}");
                }
            }
        }
    }

    let v = kodi.volume()?;
    say!(
        "volume {}{}",
        v.volume,
        if v.muted { " (muted)" } else { "" }
    );
    Ok(())
}

fn hms(secs: f64) -> String {
    let t = secs.max(0.0) as u64;
    let (h, m, s) = (t / 3600, (t / 60) % 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
