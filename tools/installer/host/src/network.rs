use crate::{
    adapter::Worker,
    frontend::{Choice, Ui},
    public_inputs::{decode, hex},
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    num::NonZeroU32,
    thread,
    time::{Duration, Instant},
};
pub fn rpc(worker: &mut Worker, operation: &str, payload: Value) -> Result<Value> {
    worker.operation(Duration::from_secs(75), |w| {
        w.send(&json!({"op":operation,"payload":payload}))?;
        let event = w.event()?;
        ensure!(event["event"] == operation, "unexpected stage setup event");
        Ok(event["result"].clone())
    })
}
/// The MediaTek gen2 driver injects a pseudo network named "NVRAM WARNING:
/// Err = 0x01" into scan results when it finds no calibration record. The RAM
/// stage deliberately runs without one, so the notice is expected there and is
/// not a network anyone can join. Hide it; never treat it as a selectable SSID.
fn driver_notice(raw: &[u8]) -> bool {
    raw.starts_with(b"NVRAM WARNING")
}
fn failure_reason(status: &Value) -> &str {
    status["error"]
        .as_str()
        .filter(|v| {
            matches!(
                *v,
                "detect-node"
                    | "loader-exit"
                    | "transport-node"
                    | "wifi-node"
                    | "launcher-exit"
                    | "transport-timeout"
                    | "power-on"
                    | "interface-timeout"
                    | "interface-up"
                    | "control-directory"
                    | "dhcp-exit"
                    | "supplicant-exit"
                    | "supplicant-socket-timeout"
            )
        })
        .unwrap_or("unknown")
}
pub fn ready(worker: &mut Worker, ui: &mut Ui) -> Result<()> {
    let start = Instant::now();
    loop {
        ensure!(
            start.elapsed() < Duration::from_secs(120),
            "Wi-Fi did not become ready"
        );
        ui.progress_with_unit(
            4,
            "Starting remote Wi-Fi",
            start.elapsed().as_secs(),
            120,
            crate::frontend::ProgressUnit::Seconds,
        )?;
        let status = rpc(worker, "stage_status", Value::Null)?;
        ensure!(
            status["provisioned"] != true,
            "Wi-Fi stage was already provisioned"
        );
        if status["status"] == "failed" {
            let reason = failure_reason(&status);
            anyhow::bail!("Remote Wi-Fi initialization failed: {reason}");
        }
        if status["status"] == "ready" {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
}
fn display(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.chars()
        .flat_map(|c| {
            if c.is_control() || matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}') {
                format!("\\u{{{:x}}}", c as u32).chars().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
pub fn credentials(ssid: &[u8], password: Option<&str>) -> Result<Value> {
    ensure!(
        !ssid.is_empty() && ssid.len() <= 32,
        "SSID must be 1–32 bytes"
    );
    let psk = if let Some(password) = password {
        ensure!(
            (8..=63).contains(&password.len()) && password.bytes().all(|c| (32..=126).contains(&c)),
            "WPA2 password must be 8–63 printable ASCII characters"
        );
        let mut key = zeroize::Zeroizing::new([0u8; 32]);
        ring::pbkdf2::derive(
            ring::pbkdf2::PBKDF2_HMAC_SHA1,
            NonZeroU32::new(4096).unwrap(),
            ssid,
            password.as_bytes(),
            key.as_mut(),
        );
        Some(hex(key.as_ref()))
    } else {
        None
    };
    Ok(json!({"ssid_hex":hex(ssid),"psk_hex":psk}))
}
pub fn select(worker: &mut Worker, ui: &mut Ui) -> Result<Value> {
    loop {
        ui.progress(4, "Scanning networks visible to your remote", 0, 0)?;
        let status = rpc(worker, "stage_status", Value::Null)?;
        ensure!(
            status["scan"] == true,
            "Installer stage lacks network scanning"
        );
        let scan = rpc(worker, "stage_scan", Value::Null)?;
        ensure!(
            matches!(scan["status"].as_str(), Some("ok" | "unavailable"))
                && scan["truncated"].is_boolean(),
            "invalid network scan response"
        );
        let scanned = scan["networks"]
            .as_array()
            .context("missing network list")?;
        ensure!(scanned.len() <= 64, "network list exceeds bound");
        // Filter before numbering so choice indices and network entries stay aligned.
        let mut networks = Vec::new();
        for n in scanned {
            let raw = decode(n["ssid_hex"].as_str().context("missing SSID")?)?;
            ensure!(!raw.is_empty() && raw.len() <= 32, "invalid scanned SSID");
            if !driver_notice(&raw) {
                networks.push(n);
            }
        }
        let mut choices = Vec::new();
        for n in &networks {
            let raw = decode(n["ssid_hex"].as_str().context("missing SSID")?)?;
            let security = n["security"].as_str().context("missing network security")?;
            ensure!(
                ["wpa2", "open", "enterprise", "wpa3", "wep", "unsupported"].contains(&security),
                "invalid security type"
            );
            let dbm = n["dbm"].as_i64().context("invalid signal level")?;
            ensure!((-127..=0).contains(&dbm), "invalid signal level");
            choices.push(Choice {
                label: format!("{} · {} · {} dBm", display(&raw), security, dbm),
                detail: if matches!(security, "wpa2" | "open") {
                    String::new()
                } else {
                    "Unsupported security".into()
                },
            });
        }
        choices.push(Choice {
            label: "Enter network manually / hidden network".into(),
            detail: "Enter the exact SSID".into(),
        });
        choices.push(Choice {
            label: "Scan again".into(),
            detail: String::new(),
        });
        let selected = ui.choose(
            "Wi-Fi for your remote",
            "Choose WPA2 Personal or an open network.",
            &choices,
        )?;
        if selected == networks.len() + 1 {
            continue;
        }
        let (ssid, open) = if selected == networks.len() {
            let ssid = ui.input(
                "Network name",
                "Enter the exact SSID, including capitalization.",
                false,
            )?;
            let open = ui.choose(
                "Network security",
                "Choose the network's security.",
                &[
                    Choice {
                        label: "WPA2 Personal".into(),
                        detail: String::new(),
                    },
                    Choice {
                        label: "Open network".into(),
                        detail: "No Wi-Fi password".into(),
                    },
                ],
            )? == 1;
            (ssid.as_bytes().to_vec(), open)
        } else {
            let n = &networks[selected];
            if !matches!(n["security"].as_str(), Some("wpa2" | "open")) {
                continue;
            }
            (
                decode(n["ssid_hex"].as_str().unwrap())?,
                n["security"] == "open",
            )
        };
        if open {
            return credentials(&ssid, None);
        }
        loop {
            let password=ui.input("Wi-Fi password","This is sent through USB to your remote and saved in its protected Wi-Fi configuration.",true)?;
            match credentials(&ssid, Some(&password)) {
                Ok(v) => return Ok(v),
                Err(_) => ui.progress(4, "Use 8–63 printable ASCII characters for WPA2", 0, 0)?,
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derives_known_wpa2_psk_without_changing_ssid_bytes() {
        assert_eq!(
            credentials(b"IEEE", Some("password")).unwrap()["psk_hex"],
            "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e"
        );
        assert_eq!(
            credentials(&[0xff, 0, 65], None).unwrap()["ssid_hex"],
            "ff0041"
        );
        assert!(credentials(b"x", Some("short")).is_err());
        assert!(credentials(&[], None).is_err());
    }
    #[test]
    fn driver_calibration_notice_is_not_a_network() {
        assert!(driver_notice(b"NVRAM WARNING: Err = 0x01"));
        assert!(driver_notice(b"NVRAM WARNING"));
        assert!(!driver_notice(b"NVRAM"));
        assert!(!driver_notice(b"home"));
        assert!(!driver_notice(b" NVRAM WARNING: Err = 0x01"));
    }
    #[test]
    fn display_cannot_inject_terminal_controls() {
        assert!(!display(b"a\x1b[2J\n").contains('\x1b'));
    }
    #[test]
    fn stalled_supplicant_reason_reaches_host_error_handling() {
        let stalled = json!({
            "status": "failed",
            "error": "supplicant-socket-timeout",
        });
        assert_eq!(failure_reason(&stalled), "supplicant-socket-timeout");
        let control_directory = json!({
            "status": "failed",
            "error": "control-directory",
        });
        assert_eq!(failure_reason(&control_directory), "control-directory");
        let unknown = json!({"status": "failed", "error": "untrusted detail"});
        assert_eq!(failure_reason(&unknown), "unknown");
    }
}
