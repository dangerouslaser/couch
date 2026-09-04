//! What the UI needs to know about the device it is running on.

use std::process::Command;

const BATTERY: &str = "/sys/class/power_supply/battery/";

pub struct Battery {
    pub percent: i32,
    pub charging: bool,
}

/// Reads the kernel's own gauge. "status" is the authoritative charging signal:
/// usb/online only reports that a cable is present, which it always is while
/// the remote sits on a bench being debugged, so it would read as charging for
/// ever. Returns None rather than inventing a figure if the gauge is missing.
pub fn battery() -> Option<Battery> {
    let percent: i32 = read_trimmed(&format!("{BATTERY}capacity"))?.parse().ok()?;
    let status = read_trimmed(&format!("{BATTERY}status")).unwrap_or_default();
    Some(Battery {
        percent: percent.clamp(0, 100),
        charging: status == "Charging" || status == "Full",
    })
}

/// Local time as "H:MM AM/PM".
///
/// The offset comes from `date`, which is a process spawn - so it is cached by
/// the caller and refreshed hourly rather than every tick.
pub fn utc_offset_seconds() -> i64 {
    let out = match Command::new("date").arg("+%z").output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => return 0,
    };
    // +HHMM
    if out.len() < 5 {
        return 0;
    }
    let sign = if out.starts_with('-') { -1 } else { 1 };
    let hours: i64 = out[1..3].parse().unwrap_or(0);
    let mins: i64 = out[3..5].parse().unwrap_or(0);
    sign * (hours * 3600 + mins * 60)
}

/// HH:MM, 24-hour, per the design's status bar.
pub fn clock_24h(offset_seconds: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        + offset_seconds;
    let secs_today = now.rem_euclid(86_400);
    format!("{:02}:{:02}", secs_today / 3600, (secs_today % 3600) / 60)
}

#[allow(dead_code)]
pub fn clock_string(offset_seconds: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        + offset_seconds;
    let secs_today = now.rem_euclid(86_400);
    let h24 = secs_today / 3600;
    let minute = (secs_today % 3600) / 60;
    let suffix = if h24 < 12 { "AM" } else { "PM" };
    let h12 = match h24 % 12 {
        0 => 12,
        h => h,
    };
    format!("{h12}:{minute:02} {suffix}")
}

/// Setup mode is a marker file, not an environment variable: the GUI is
/// supervised in a restart loop whose environment is fixed at boot, so an env
/// flag could never be cleared while running - which is exactly what joining a
/// network has to do.
pub fn in_setup_mode() -> bool {
    std::path::Path::new("/tmp/couch.setup").exists()
}

pub fn setup_ssid() -> String {
    read_trimmed("/tmp/portal.ssid").unwrap_or_else(|| "Couch-Setup".into())
}

/// The portal asks for a physical button press before granting SSH access, and
/// confirm.sh has no way to say so on the panel - its output goes to a log.
pub enum Approval {
    Idle,
    Waiting,
    Granted,
    TimedOut,
}

pub fn approval_state() -> Approval {
    if std::path::Path::new("/tmp/press.request").exists() {
        return Approval::Waiting;
    }
    match read_trimmed("/tmp/press.result").as_deref() {
        Some("ok") => Approval::Granted,
        Some("timeout") => Approval::TimedOut,
        _ => Approval::Idle,
    }
}

/// The four digits couch-confd wants shown, if a browser is trying to pair.
///
/// The daemon deletes the file when the PIN is spent or refused, but it has no
/// timer of its own - expiry is noticed on the next request - so a PIN nobody
/// follows up on can outlive its usefulness on disk. The caller runs its own
/// countdown over what it reads here rather than trusting the file to vanish.
pub fn pairing_pin() -> Option<String> {
    let pin = read_trimmed("/tmp/couch.pin")?;
    // Anything else is a file we did not write.
    (pin.len() == 4 && pin.bytes().all(|b| b.is_ascii_digit())).then_some(pin)
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}
