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

/// Wi-Fi signal, as bars: 0 is disconnected, 1 through 4 climb with the RSSI.
///
/// `operstate` is the association signal - it reads "up" only once the link
/// layer is associated and the carrier is on, and drops to "down"/"dormant"
/// the moment the radio loses the AP - so it, not the mere presence of the
/// interface, decides connected from not. The strength comes from the driver's
/// own line in /proc/net/wireless, whose fourth field is the level in dBm
/// (e.g. "-63."). Read from files, not `iw` or `wpa_cli`: those are process
/// spawns, and this runs every second.
pub fn wifi_level() -> i32 {
    if read_trimmed("/sys/class/net/wlan0/operstate").as_deref() != Some("up") {
        return 0;
    }
    let dbm = std::fs::read_to_string("/proc/net/wireless").ok().and_then(|s| {
        s.lines()
            .find(|l| l.trim_start().starts_with("wlan0:"))
            .and_then(|l| l.split_whitespace().nth(3))
            .map(|t| t.trim_end_matches('.').to_string())
            .and_then(|t| t.parse::<f32>().ok())
    });
    match dbm {
        Some(d) if d >= -55.0 => 4,
        Some(d) if d >= -67.0 => 3,
        Some(d) if d >= -78.0 => 2,
        // Associated but weak, or associated with no readable level: still
        // connected, so never 0 here - 0 means down, and down is the one
        // state a person needs to see at a glance.
        Some(_) | None => 1,
    }
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

/// The four digits couch-confd wants shown, and how many seconds they have
/// left, if a browser is trying to pair.
///
/// The deadline comes out of the file rather than being counted from when this
/// process first saw it. Timing it locally meant a `couch-gui` restart handed a
/// stale PIN a fresh two minutes - which, with a daemon that only expired the
/// challenge when a request arrived, left the panel stuck on a PIN forever.
/// Both processes read the same clock, so an absolute deadline agrees between
/// them even if the device's clock is wrong.
pub fn pairing_pin() -> Option<(String, i32)> {
    let raw = read_trimmed("/tmp/couch.pin")?;
    let mut parts = raw.split_whitespace();
    let pin = parts.next()?.to_string();
    // Anything else is a file we did not write.
    if pin.len() != 4 || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let deadline: i64 = parts.next()?.parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some((pin, (deadline - now).clamp(0, 3600) as i32))
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

// --- user settings ----------------------------------------------------------
//
// Brightness and the two standby timeouts, chosen in the settings menu and
// kept across restarts in one small file. Written atomically (temp then
// rename) so a crash mid-write cannot leave a half-line the parser trips on;
// read leniently, since a file a person never opened is the defaults.

/// Dim-after choices, seconds and their labels, index-aligned with the menu.
pub const DIM_SECS: [u64; 5] = [15, 30, 60, 120, 300];
pub const DIM_LABELS: [&str; 5] = ["15s", "30s", "1m", "2m", "5m"];
/// Screen-off choices; 0 is "Never", the one that only dims.
pub const OFF_SECS: [u64; 6] = [30, 60, 120, 300, 600, 0];
pub const OFF_LABELS: [&str; 6] = ["30s", "1m", "2m", "5m", "10m", "Never"];

const SETTINGS_PATH: &str = "/opt/couch/settings.conf";

pub struct UiSettings {
    pub brightness: i32, // 10..100, percent
    pub dim_index: i32,
    pub off_index: i32,
}

impl Default for UiSettings {
    fn default() -> Self {
        // 100% bright, dim after 30s, off after 2m - the timings that were
        // hard-coded before the menu existed.
        UiSettings { brightness: 100, dim_index: 1, off_index: 3 }
    }
}

pub fn load_settings() -> UiSettings {
    let mut s = UiSettings::default();
    if let Ok(text) = std::fs::read_to_string(SETTINGS_PATH) {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "brightness" => {
                    if let Ok(n) = v.parse::<i32>() {
                        s.brightness = n.clamp(10, 100);
                    }
                }
                "dim" => {
                    if let Ok(n) = v.parse::<i32>() {
                        s.dim_index = n.clamp(0, DIM_SECS.len() as i32 - 1);
                    }
                }
                "off" => {
                    if let Ok(n) = v.parse::<i32>() {
                        s.off_index = n.clamp(0, OFF_SECS.len() as i32 - 1);
                    }
                }
                _ => {}
            }
        }
    }
    s
}

pub fn save_settings(s: &UiSettings) {
    let body = format!(
        "brightness={}\ndim={}\noff={}\n",
        s.brightness, s.dim_index, s.off_index
    );
    let tmp = format!("{SETTINGS_PATH}.tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, SETTINGS_PATH);
    }
}

/// Percent to an 8-bit backlight level, with a floor: 10% must still be
/// visible, not off. 0 is reserved for standby power-down.
pub fn brightness_level(percent: i32) -> u8 {
    let p = percent.clamp(10, 100) as u32;
    // 10% -> ~30, 100% -> 255, linear between.
    (30 + (p - 10) * (255 - 30) / 90) as u8
}

/// The SSID the radio is on, or "" if it is not associated. A process spawn -
/// only called when the settings menu opens, never on the tick.
pub fn wifi_ssid() -> String {
    if let Ok(out) = Command::new("iwgetid").args(["-r", "wlan0"]).output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    // Fallback: wpa_cli against the socket stage2 opens.
    if let Ok(out) = Command::new("wpa_cli")
        .args(["-p", "/tmp/wpa", "-i", "wlan0", "status"])
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(v) = line.strip_prefix("ssid=") {
                return v.trim().to_string();
            }
        }
    }
    String::new()
}

/// Whether sshd is listening.
pub fn ssh_running() -> bool {
    std::fs::read_dir("/proc")
        .map(|dir| {
            dir.filter_map(|e| e.ok()).any(|e| {
                std::fs::read_to_string(e.path().join("comm"))
                    .map(|c| c.trim() == "sshd")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Whether anyone is enrolled to use SSH: a key, or a root password. Without
/// one, sshd would listen for nobody, so the toggle offers nothing to turn on.
pub fn ssh_available() -> bool {
    let has_key = std::fs::read_to_string("/root/.ssh/authorized_keys")
        .map(|s| s.lines().any(|l| !l.trim().is_empty()))
        .unwrap_or(false);
    let has_pw = std::fs::read_to_string("/etc/shadow")
        .map(|s| {
            s.lines()
                .filter(|l| l.starts_with("root:"))
                .any(|l| {
                    let f = l.split(':').nth(1).unwrap_or("");
                    !f.is_empty() && f != "!" && f != "*" && f != "!!"
                })
        })
        .unwrap_or(false);
    has_key || has_pw
}
