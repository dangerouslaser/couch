//! What the UI needs to know about the device it is running on.

use std::process::Command;

/// Only called after a real GUI frame, then from the functioning main loop.
/// Atomic replacement keeps init from reading half a heartbeat.
pub fn report_gui_health() -> std::io::Result<()> {
    // /proc/uptime includes system suspend, matching init's freshness clock.
    let uptime = std::fs::read_to_string("/proc/uptime")?;
    let seconds = uptime
        .split('.')
        .next()
        .unwrap_or("")
        .parse::<u64>()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let pid = std::process::id();
    let temporary = format!("/tmp/couch-gui.health.{pid}");
    std::fs::write(&temporary, format!("{pid} {seconds}\n"))?;
    std::fs::rename(temporary, "/tmp/couch-gui.health")
}

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
    let dbm = wifi_dbm();
    match dbm {
        Some(d) if d >= -55 => 4,
        Some(d) if d >= -67 => 3,
        Some(d) if d >= -78 => 2,
        // Associated but weak, or associated with no readable level: still
        // connected, so never 0 here - 0 means down, and down is the one
        // state a person needs to see at a glance.
        Some(_) | None => 1,
    }
}

/// RSSI from the MT6580 driver's wireless statistics, not a guessed bar count.
pub fn wifi_dbm() -> Option<i32> {
    std::fs::read_to_string("/proc/net/wireless")
        .ok()
        .and_then(|text| crate::wifi::signal_dbm(&text))
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

/// The wpa_supplicant control socket stage2 opens, and the tools that talk to
/// it. Absolute, because couch-gui's PATH at boot is not guaranteed to carry
/// /sbin.
const WPA_CLI: &str = "/sbin/wpa_cli";
const WPA_CTRL: &str = "/tmp/wpa";
const SSHD: &str = "/usr/sbin/sshd";

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
    /// Whether SSH should be running. The preference, persisted so a reboot
    /// keeps it; sshd.sh reads it at boot. Only meaningful when a key or
    /// password is enrolled - see ssh_available.
    pub ssh: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        // 100% bright, dim after 30s, off after 2m - the timings that were
        // hard-coded before the menu existed.
        // SSH default follows enrolment: if a key is enrolled the shipped
        // sshd.sh already starts it, so the stored default matches.
        UiSettings { brightness: 100, dim_index: 1, off_index: 3, ssh: ssh_available() }
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
                "ssh" => s.ssh = v == "1",
                _ => {}
            }
        }
    }
    s
}

pub fn save_settings(s: &UiSettings) {
    let body = format!(
        "brightness={}\ndim={}\noff={}\nssh={}\n",
        s.brightness, s.dim_index, s.off_index, if s.ssh { 1 } else { 0 }
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

/// Cached supplicant status, refreshed in a worker without spawning tools or
/// blocking rendering. An unreadable network name does not imply link loss.
pub fn wifi_ssid() -> String {
    crate::wifi::status().ssid
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

// --- SSH control ------------------------------------------------------------
//
// Start and stop sshd at runtime. Enrolment - a key or a root password - stays
// the setup portal's job, gated on a physical button press; this only runs or
// stops the daemon for someone who is already enrolled and is, by holding the
// remote to reach this menu, physically present.

/// Start sshd. Generates host keys on first use (never baked into the image,
/// or every device would share them). Returns whether sshd is listening after.
pub fn ssh_start() -> bool {
    if !ssh_available() {
        return false;
    }
    if !std::path::Path::new("/etc/ssh/ssh_host_ed25519_key").exists() {
        let _ = Command::new("/usr/bin/ssh-keygen").arg("-A").output();
    }
    let _ = Command::new(SSHD).output();
    ssh_running()
}

/// Stop sshd. Returns whether it is stopped after.
pub fn ssh_stop() -> bool {
    // pkill is not guaranteed present; kill by pid from /proc, the same scan
    // ssh_running uses.
    if let Ok(dir) = std::fs::read_dir("/proc") {
        for e in dir.filter_map(|e| e.ok()) {
            let is_sshd = std::fs::read_to_string(e.path().join("comm"))
                .map(|c| c.trim() == "sshd")
                .unwrap_or(false);
            if is_sshd {
                if let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) {
                    unsafe { libc::kill(pid, libc::SIGTERM) };
                }
            }
        }
    }
    !ssh_running()
}

// --- Wi-Fi reconfiguration --------------------------------------------------
//
// Add a network to the running wpa_supplicant and select it, then persist it so
// it survives a reboot. Association is asynchronous - wpa_supplicant takes a few
// seconds - so this only fires the commands; the caller polls `wifi_ssid` to see
// whether it took.

fn wpa(args: &[&str]) -> Option<String> {
    let out = Command::new(WPA_CLI)
        .args(["-p", WPA_CTRL, "-i", "wlan0"])
        .args(args)
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Configure and select a network. An empty passphrase means an open network.
/// Returns false if wpa_supplicant is not reachable at all.
pub fn wifi_connect(ssid: &str, passphrase: &str) -> bool {
    // wpa_cli quoting: the value is wrapped in literal double quotes, and an
    // SSID or passphrase containing a quote would break it - reject rather than
    // build a broken config. The keyboard can produce quotes, so this matters.
    if ssid.is_empty() || ssid.contains('"') || passphrase.contains('"') {
        return false;
    }
    // COUCH_WIFI_DRYRUN prints the plan and changes nothing: selecting a new
    // network drops the current association, which cannot be tested over the
    // very link it would drop. The password is not printed.
    if std::env::var_os("COUCH_WIFI_DRYRUN").is_some() {
        println!("couch-gui: wifi_connect DRYRUN ssid={ssid:?} open={}", passphrase.is_empty());
        println!("couch-gui:   add_network; set ssid; set {}; enable; select; save; udhcpc",
                 if passphrase.is_empty() { "key_mgmt NONE" } else { "psk ***" });
        return true;
    }
    let Some(id) = wpa(&["add_network"]) else { return false };
    let id = id.lines().last().unwrap_or("").trim();
    if id.is_empty() || id == "FAIL" {
        return false;
    }
    let _ = wpa(&["set_network", id, "ssid", &format!("\"{ssid}\"")]);
    if passphrase.is_empty() {
        let _ = wpa(&["set_network", id, "key_mgmt", "NONE"]);
    } else {
        let _ = wpa(&["set_network", id, "psk", &format!("\"{passphrase}\"")]);
    }
    let _ = wpa(&["enable_network", id]);
    let _ = wpa(&["select_network", id]);
    let _ = wpa(&["save_config"]);
    persist_network(ssid, passphrase);
    // A fresh association needs a fresh lease; run udhcpc detached so it does
    // not block the UI. -n gives up if no server answers rather than looping.
    let _ = Command::new("/sbin/udhcpc")
        .args(["-i", "wlan0", "-n", "-q", "-t", "10", "-b"])
        .spawn();
    true
}

/// Append the network to /opt/couch/networks.conf, which stage2 merges at boot.
/// Two lines, `ssid` and `psk`, the format wifi-conf.sh already reads.
fn persist_network(ssid: &str, passphrase: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/opt/couch/networks.conf")
    {
        let _ = writeln!(f, "ssid={ssid}");
        let _ = writeln!(f, "psk={passphrase}");
    }
}

/// wpa_supplicant's association state, e.g. "COMPLETED", "SCANNING",
/// "4WAY_HANDSHAKE". "" if it cannot be read.
pub fn wifi_state() -> String {
    crate::wifi::status().state
}
