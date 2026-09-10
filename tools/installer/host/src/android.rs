//! Read-only ADB enrollment and one explicitly selected reboot. USB topology and
//! official image/layout admission remain the caller's independent obligations.
use anyhow::{ensure, Context, Result};
use std::{
    fs::File,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const LIMIT: u64 = 65536;
const DEADLINE: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AndroidDevice {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
}
#[derive(Clone, Debug)]
pub struct AndroidIdentity {
    pub serial: String,
    pub model: String,
    pub device_id: Option<String>,
    pub wifi_mac: Option<String>,
    pub bt_mac: Option<String>,
    pub cid: Option<String>,
    pub usb_bus: Option<u16>,
    pub usb_ports: Option<Vec<u8>>,
}
fn serial_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}

// Redirect output to a private file: no blocked reader thread can outlive a
// timed-out ADB client. stderr is deliberately discarded (may contain IDs).
fn command(adb: &Path, args: &[&str], deadline: Duration) -> Result<Option<String>> {
    let mut output = tempfile::tempfile()?;
    let mut child = Command::new(adb)
        .args(args)
        .current_dir(
            adb.parent()
                .context("ADB executable must have a parent directory")?,
        )
        .env_remove("ADB_SERVER_SOCKET")
        .env_remove("ANDROID_ADB_SERVER_PORT")
        .env_remove("ANDROID_ADB_SERVER_ADDRESS")
        .env_remove("ANDROID_SERIAL")
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start the verified ADB executable")?;
    let started = Instant::now();
    let result = (|| loop {
        ensure!(started.elapsed() < deadline, "ADB query timed out");
        ensure!(
            output.metadata()?.len() <= LIMIT,
            "ADB output exceeded limit"
        );
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Ok(None);
            }
            return read_output(&mut output).map(Some);
        }
        thread::sleep(Duration::from_millis(10));
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
fn read_output(output: &mut File) -> Result<String> {
    use std::io::{Seek, SeekFrom};
    output.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    output.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "ADB output exceeded limit");
    String::from_utf8(bytes).context("ADB returned invalid text")
}
fn parse_devices(text: &str) -> Result<Vec<AndroidDevice>> {
    let mut devices = Vec::new();
    for line in text.lines().map(str::trim).filter(|s| !s.is_empty()) {
        if line == "List of devices attached" {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        ensure!(
            fields.len() >= 2 && serial_valid(fields[0]),
            "Invalid ADB device listing"
        );
        // Network ADB is excluded: physical USB binding is mandatory later.
        if fields[0].contains(':') {
            continue;
        }
        ensure!(
            !devices
                .iter()
                .any(|d: &AndroidDevice| d.serial == fields[0]),
            "Duplicate ADB serial"
        );
        devices.push(AndroidDevice {
            serial: fields[0].into(),
            state: fields[1].into(),
            model: fields
                .iter()
                .find_map(|s| s.strip_prefix("model:"))
                .map(str::to_owned),
        });
    }
    Ok(devices)
}
pub fn discover(adb: &Path) -> Result<Vec<AndroidDevice>> {
    parse_devices(
        &command(adb, &["devices", "-l"], DEADLINE)?.context("ADB device discovery failed")?,
    )
}
fn authorized(adb: &Path, serial: &str) -> Result<()> {
    ensure!(
        serial_valid(serial) && !serial.contains(':'),
        "Invalid physical ADB serial"
    );
    ensure!(
        discover(adb)?
            .iter()
            .any(|d| d.serial == serial && d.state == "device"),
        "Enable USB debugging and authorize the selected remote"
    );
    Ok(())
}
fn mac(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    let bytes: Option<Vec<u8>> = value
        .split(':')
        .map(|v| {
            if v.len() != 2 {
                None
            } else {
                u8::from_str_radix(v, 16).ok()
            }
        })
        .collect();
    let bytes = bytes?;
    if bytes.len() != 6
        || bytes[0] & 1 != 0
        || bytes.iter().all(|b| *b == 0)
        || value == "02:00:00:00:00:00"
    {
        return None;
    }
    Some(value)
}
fn canonical_cid(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    (value.len() == 32
        && value.bytes().all(|b| b.is_ascii_hexdigit())
        && value != "0".repeat(32)
        && value != "f".repeat(32))
    .then_some(value)
}
pub fn capture(adb: &Path, serial: &str) -> Result<AndroidIdentity> {
    authorized(adb, serial)?;
    let query = |words: &[&str]| -> Result<Option<String>> {
        let mut args = vec!["-s", serial, "shell"];
        args.extend_from_slice(words);
        command(adb, &args, DEADLINE)
    };
    let model = query(&["getprop", "ro.product.model"])?
        .unwrap_or_default()
        .trim()
        .to_owned();
    ensure!(
        model.len() <= 128 && !model.chars().any(char::is_control),
        "Invalid Android model response"
    );
    Ok(AndroidIdentity {
        serial: serial.into(),
        model,
        // Vendor Device ID is not Android ID or ro.serialno. No verified
        // property mapping exists; the TUI must ask for the recorded value.
        device_id: None,
        wifi_mac: mac(query(&["cat", "/sys/class/net/wlan0/address"])?),
        bt_mac: mac(query(&["settings", "get", "secure", "bluetooth_address"])?),
        cid: canonical_cid(query(&["cat", "/sys/block/mmcblk0/device/cid"])?),
        usb_bus: None,
        usb_ports: None,
    })
}
/// Call only after binding this serial to the retained physical USB port and CID.
/// An ambiguous failure is never retried: the remote may already have restarted.
pub fn reboot(adb: &Path, serial: &str) -> Result<()> {
    authorized(adb, serial)?;
    command(adb, &["-s", serial, "reboot"], DEADLINE)?
        .context("ADB reboot was not acknowledged; check the remote before retrying")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_preserves_unauthorized_and_excludes_network() {
        let devices = parse_devices("List of devices attached\nabc unauthorized usb:1-2\nxyz device model:HA100 transport_id:2\n192.168.1.2:5555 device\n").unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].state, "unauthorized");
        assert_eq!(devices[1].model.as_deref(), Some("HA100"));
        assert!(parse_devices("abc device\nabc device").is_err());
        assert!(parse_devices("$(bad) device").is_err());
    }
    #[test]
    fn missing_and_placeholder_identity_is_never_inferred() {
        for value in [
            "",
            "null",
            "02:00:00:00:00:00",
            "00:00:00:00:00:00",
            "ff:ff:ff:ff:ff:ff",
            "01:23:45:67:89:ab",
        ] {
            assert_eq!(mac(Some(value.into())), None);
        }
        assert_eq!(
            mac(Some(" A0:B1:C2:D3:E4:F5\n".into())).as_deref(),
            Some("a0:b1:c2:d3:e4:f5")
        );
        assert!(canonical_cid(Some("0".repeat(32))).is_none());
        assert!(canonical_cid(Some("F".repeat(32))).is_none());
        assert_eq!(
            canonical_cid(Some("A1234567890123456789012345678901\n".into())).as_deref(),
            Some("a1234567890123456789012345678901")
        );
    }
    #[cfg(unix)]
    #[test]
    fn actual_child_timeout_and_excess_output_are_bounded() {
        let started = Instant::now();
        assert!(command(
            Path::new("/bin/sh"),
            &["-c", "exec sleep 5"],
            Duration::from_millis(100)
        )
        .is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(command(
            Path::new("/bin/sh"),
            &[
                "-c",
                "while :; do printf '0123456789012345678901234567890123456789'; done"
            ],
            Duration::from_secs(2)
        )
        .is_err());
    }
}
