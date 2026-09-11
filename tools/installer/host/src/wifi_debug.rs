//! Direct attachment to a RAM Wi-Fi stage. No installer workflow is invoked.
use crate::{
    adapter::{UsbLease, Worker},
    dependencies,
    frontend::{Choice, Ui},
    network,
    public_inputs::{create, hex},
    session::{self, SessionGuard},
};
use anyhow::{ensure, Context, Result};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    schema: u32,
    bus: u8,
    ports: Vec<u8>,
    expected_stage: String,
    runtime_root: PathBuf,
    runtime_receipt_sha256: String,
    #[serde(default)]
    transition: Option<crate::wifi_debug_transition::Config>,
    #[serde(default)]
    completed_transition: Option<crate::wifi_debug_transition::ReceiptConfig>,
}
impl Config {
    fn validate(&self) -> Result<()> {
        if self.expected_stage == "wifi-debug-v1" {
            ensure!(self.transition.is_some() != self.completed_transition.is_some(),
                "debug stage requires exactly one new transition or pinned completed transition receipt");
        } else {
            ensure!(
                self.transition.is_none() && self.completed_transition.is_none(),
                "legacy status cannot use a debug transition or receipt"
            );
        }
        if let Some(receipt) = &self.completed_transition {
            receipt.validate()?;
        }
        if let Some(transition) = &self.transition {
            ensure!(
                self.expected_stage == "wifi-debug-v1",
                "transition requires dedicated debug stage mode"
            );
            transition.validate()?;
        }
        ensure!(
            self.schema == 1
                && self.bus != 0
                && (1..=7).contains(&self.ports.len())
                && self.ports.iter().all(|n| *n != 0),
            "invalid debug configuration or USB topology"
        );
        ensure!(
            self.runtime_root.is_absolute(),
            "debug runtime path must be absolute"
        );
        ensure!(
            matches!(
                self.expected_stage.as_str(),
                "legacy-status" | "wifi-debug-v1"
            ),
            "expected_stage must be legacy-status or wifi-debug-v1"
        );
        ensure!(
            self.runtime_receipt_sha256.len() == 64
                && self
                    .runtime_receipt_sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "invalid independently recorded runtime receipt SHA-256"
        );
        Ok(())
    }
}

fn materialize(session: &SessionGuard) -> Result<PathBuf> {
    for (name, bytes) in [
        (
            "wifi_debug_worker.py",
            include_bytes!("../../wifi_debug_worker.py").as_slice(),
        ),
        (
            "stage_usb.py",
            include_bytes!("../../stage_usb.py").as_slice(),
        ),
        (
            "couch_install.py",
            include_bytes!("../../couch_install.py").as_slice(),
        ),
    ] {
        let mut file = create(&session.path().join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    Ok(session.path().join("wifi_debug_worker.py"))
}

/// Copy only fixed, credential-free status fields into the debug receipt/UI.
fn status_summary(value: &Value) -> Result<Value> {
    let state = value["status"].as_str().context("missing stage status")?;
    ensure!(
        matches!(
            state,
            "waiting" | "initializing" | "ready" | "connecting" | "connected" | "failed"
        ),
        "invalid stage status"
    );
    let provisioned = value["provisioned"]
        .as_bool()
        .context("missing stage credential state")?;
    let error = value["error"].as_str().unwrap_or("unknown");
    let error = if matches!(
        error,
        "none"
            | "unknown"
            | "detect-node"
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
            | "debug-retry-limit"
    ) {
        error
    } else {
        "unknown"
    };
    Ok(json!({"status":state,"provisioned":provisioned,"error":error}))
}

fn require_debug_identity(status: &Value) -> Result<()> {
    ensure!(
        status["wifi_debug"] == true
            && status["capabilities"] == "COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1"
            && status["stage_kind"] == "private-ram-wifi-debug-stage"
            && status["debug_protocol"].as_u64() == Some(1)
            && status["scan"] == false
            && status["debug_generation_limit"].as_u64() == Some(8)
            && status["provisioned"] == false,
        "Expected dedicated unprovisioned Wi-Fi debug stage; transition the stage before retrying"
    );
    Ok(())
}

fn diagnostic_summary(value: &Value) -> Result<Value> {
    ensure!(
        value["stage_kind"] == "private-ram-wifi-debug-stage"
            && value["capability"] == "COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1"
            && value["debug_protocol"].as_u64() == Some(1)
            && value["debug_generation_limit"].as_u64() == Some(8)
            && value["precredential"] == true,
        "expected versioned pre-credential debug diagnostics"
    );
    require_debug_identity(&value["status"])?;
    let status = status_summary(&value["status"])?;
    ensure!(
        status["provisioned"] == false,
        "debug stage received credentials"
    );
    let generation = value["generation"]
        .as_u64()
        .filter(|v| *v <= 8)
        .context("invalid diagnostic generation")?;
    let step = value["step"].as_str().context("missing diagnostic step")?;
    ensure!(
        matches!(
            step,
            "unknown" | "detect" | "loader" | "transport" | "power"
        ),
        "invalid diagnostic step"
    );
    let log = value["log"].as_str().context("missing diagnostic log")?;
    ensure!(log.len() <= 4096, "diagnostic log exceeds bound");
    let log: String = log
        .chars()
        .filter(|c| {
            (!c.is_control() || *c == '\n' || *c == '\t')
                && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    Ok(
        json!({"status":status,"generation":generation,"debug_generation_limit":8,"step":step,"precredential":true,"log":log}),
    )
}

pub fn run(ui: &mut Ui, path: &Path) -> Result<()> {
    let mut bytes = Vec::new();
    crate::regular(path)?.take(16385).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16384, "debug configuration exceeds bound");
    let config: Config = serde_json::from_slice(&bytes)?;
    config.validate()?;
    ui.set_steps(
        ["Runtime", "Attach", "Status", "Diagnostics", "Wi-Fi"]
            .map(String::from)
            .to_vec(),
    )?;
    // Same root/lease as the normal native installer, never a caller-selected
    // temporary lock directory that could bypass an active installer session.
    #[cfg(windows)]
    let parent = PathBuf::from(std::env::var_os("LOCALAPPDATA").context("missing app data")?)
        .join("CouchInstaller");
    #[cfg(not(windows))]
    let parent = PathBuf::from(std::env::var_os("HOME").context("missing home directory")?)
        .join(".couch-installer");
    if !parent.exists() {
        session::create_private_parent(&parent)?;
    }
    let mut nonce = [0; 16];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("secure randomness unavailable"))?;
    let mut session = SessionGuard::create(&parent.join(format!("wifi-debug-{}", hex(&nonce))))?;
    let _lease = UsbLease::acquire(&session)?;
    ui.set_log_path(session.path().to_str().context("invalid session path")?)?;
    ui.progress(0, "Verifying the existing debug runtime", 0, 0)?;
    let (python, libusb) =
        dependencies::debug_runtime(&config.runtime_root, &config.runtime_receipt_sha256)?;
    let verifier = if config.expected_stage == "wifi-debug-v1" {
        Some(crate::wifi_debug_transition::materialize(&session)?)
    } else {
        None
    };
    let receipt = if let Some(transition) = &config.transition {
        let transition_result = crate::wifi_debug_transition::run(
            ui,
            &mut session,
            transition,
            crate::wifi_debug_transition::Environment {
                python: &python,
                runtime: &config.runtime_root,
                libusb: &libusb,
                script: verifier.as_ref().context("missing receipt verifier")?,
            },
            config.bus,
            &config.ports,
        );
        if transition_result.is_err() {
            let _ = session
                .checkpoint(&json!({"event":"debug_transition_stopped","preserve_originals":true}));
        }
        match transition_result? {
            Some(receipt) => Some(receipt),
            None => {
                session.checkpoint(&json!({"event":"debug_transition_cancelled"}))?;
                return Ok(());
            }
        }
    } else {
        config.completed_transition.clone()
    };
    // Mandatory for both fresh and later debug attachments, before debug_open
    // can import PyUSB or enumerate/claim anything. Legacy status stays read-only.
    if config.expected_stage == "wifi-debug-v1" {
        crate::wifi_debug_transition::validate_receipt(
            ui,
            &mut session,
            receipt
                .as_ref()
                .context("missing completed transition receipt")?,
            &python,
            verifier.as_ref().context("missing receipt verifier")?,
            config.bus,
            &config.ports,
        )?;
        let mut reattach = serde_json::to_value(&config)?;
        reattach["transition"] = Value::Null;
        reattach["completed_transition"] = serde_json::to_value(&receipt)?;
        let attach_path = session.path().join("debug-attach.json");
        let mut attach_file = create(&attach_path)?;
        attach_file.write_all(&serde_json::to_vec_pretty(&reattach)?)?;
        attach_file.sync_all()?;
        session.checkpoint(
            &json!({"event":"debug_reattach_configuration_saved","file":attach_path}),
        )?;
    }
    let script = materialize(&session)?;
    session.checkpoint(
        &json!({"event":"debug_attach_requested","bus":config.bus,"ports":config.ports,
        "runtime_receipt_sha256":config.runtime_receipt_sha256,"expected_stage":config.expected_stage,"storage_workflow":false}),
    )?;
    let mut command = Command::new(python);
    command
        .args(["-I", "-B"])
        .arg(dependencies::python_path(&script)?)
        .current_dir(session.path());
    let mut worker = Worker::spawn(&mut command)?;
    let result = (|| {
        network::rpc(
            &mut worker,
            "debug_open",
            json!({"bus":config.bus,"ports":config.ports,"wait_seconds":if config.transition.is_some() {60} else {0},
            "libusb":dependencies::python_path(&libusb)?}),
        )?;
        loop {
            let raw = network::rpc(&mut worker, "stage_status", Value::Null)?;
            let status = status_summary(&raw)?;
            session.checkpoint(&json!({"event":"debug_status","result":status}))?;
            if config.expected_stage == "wifi-debug-v1" {
                require_debug_identity(&raw)?;
                let diagnostic =
                    diagnostic_summary(&network::rpc(&mut worker, "debug_status", Value::Null)?)?;
                session
                    .checkpoint(&json!({"event":"wifi_startup_diagnostic","result":diagnostic}))?;
                let can_retry = diagnostic["generation"].as_u64().unwrap() < 8
                    && diagnostic["status"]["error"] != "debug-retry-limit";
                let mut choices = vec![Choice {
                    label: "Refresh diagnostics".into(),
                    detail: "Read bounded pre-credential evidence".into(),
                }];
                if can_retry {
                    choices.push(Choice {
                        label: "Retry Wi-Fi startup".into(),
                        detail: "Run the debug stage's bounded RAM-only retry".into(),
                    });
                }
                choices.push(Choice {
                    label: "Close debug attachment".into(),
                    detail: "Preserve this session".into(),
                });
                let choice = ui.choose(
                    "Wi-Fi debug stage",
                    &format!(
                        "Attempt {} of 8; Wi-Fi {}; step {}; reason {}.\n{}",
                        diagnostic["generation"],
                        diagnostic["status"]["status"].as_str().unwrap(),
                        diagnostic["step"].as_str().unwrap(),
                        diagnostic["status"]["error"].as_str().unwrap(),
                        diagnostic["log"].as_str().unwrap()
                    ),
                    &choices,
                )?;
                if choice == choices.len() - 1 {
                    break;
                }
                if choice == 1 {
                    session.checkpoint(&json!({"event":"wifi_retry_requested","generation":diagnostic["generation"]}))?;
                    network::rpc(&mut worker, "debug_retry", Value::Null)?;
                    session.checkpoint(&json!({"event":"wifi_retry_accepted","generation":diagnostic["generation"]}))?;
                }
                continue;
            }
            let body = format!("Wi-Fi: {}; startup reason: {}; credentials received: {}.\nThis stage has no debug restart command. A debug-stage transition is required to restart Wi-Fi.",
                status["status"].as_str().unwrap(), status["error"].as_str().unwrap(), status["provisioned"]);
            let choice = ui.choose(
                "Attached RAM stage",
                &body,
                &[
                    Choice {
                        label: "Refresh status".into(),
                        detail: "Read the existing stage status".into(),
                    },
                    Choice {
                        label: "Close debug attachment".into(),
                        detail: "Preserve this session and existing backups".into(),
                    },
                ],
            )?;
            if choice == 1 {
                break;
            }
        }
        network::rpc(&mut worker, "debug_close", Value::Null)?;
        session.checkpoint(&json!({"event":"debug_closed","storage_workflow":false}))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = session.checkpoint(&json!({"event":"debug_stopped","storage_workflow":false}));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_identity_and_diagnostics_are_required_before_actions() {
        assert!(require_debug_identity(&json!({"status":"ready","provisioned":false})).is_err());
        let mut status = json!({"status":"failed","provisioned":false,"wifi_debug":true,
            "stage_kind":"private-ram-wifi-debug-stage","debug_protocol":1,"scan":false,
            "capabilities":"COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1", "debug_generation_limit":8});
        assert!(require_debug_identity(&status).is_ok());
        let mut diagnostic = json!({"status":status,"generation":1,"step":"power",
            "stage_kind":"private-ram-wifi-debug-stage", "capability":"COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1", "debug_protocol":1,
            "precredential":true,"debug_generation_limit":8,"log":"message\u{001b}\u{202e}\nnext"});
        assert_eq!(
            diagnostic_summary(&diagnostic).unwrap()["log"],
            "message\nnext"
        );
        diagnostic["log"] = json!("é".repeat(2048));
        assert!(diagnostic_summary(&diagnostic).is_ok());
        diagnostic["log"] = json!(format!("{}x", "é".repeat(2048)));
        assert!(diagnostic_summary(&diagnostic).is_err());
        status["provisioned"] = json!(true);
        assert!(require_debug_identity(&status).is_err());
    }
    #[test]
    fn generic_or_mismatched_diagnostic_schema_is_rejected() {
        let diagnostic = json!({"stage_kind":"private-ram-wifi-debug-stage",
            "capability":"COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1", "debug_protocol":1,
            "precredential":true, "generation":1, "debug_generation_limit":8, "step":"power", "log":"",
            "status":{"status":"ready", "provisioned":false, "wifi_debug":true,
                "stage_kind":"private-ram-wifi-debug-stage","debug_protocol":1,"scan":false,
                "capabilities":"COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1", "debug_generation_limit":8}});
        assert!(diagnostic_summary(&diagnostic).is_ok());
        for key in [
            "stage_kind",
            "capability",
            "debug_protocol",
            "precredential",
        ] {
            let mut invalid = diagnostic.clone();
            invalid.as_object_mut().unwrap().remove(key);
            assert!(diagnostic_summary(&invalid).is_err());
        }
        for (key, replacement) in [
            ("stage_kind", json!("benchmark")),
            ("capability", json!("generic")),
            ("debug_protocol", json!(true)),
            ("debug_protocol", json!(2)),
            ("debug_generation_limit", json!(9)),
            ("generation", json!(9)),
            ("precredential", json!(false)),
            ("step", json!("credentials")),
            ("status", json!({"status":"ready", "provisioned":false})),
        ] {
            let mut invalid = diagnostic.clone();
            invalid[key] = replacement;
            assert!(diagnostic_summary(&invalid).is_err());
        }
    }
    #[test]
    fn status_receipt_drops_untrusted_and_private_fields() {
        let result = status_summary(&json!({"status":"failed","error":"password text",
            "provisioned":false,"ssid":"private","token":"secret"}))
        .unwrap();
        assert_eq!(
            result,
            json!({"status":"failed","error":"unknown","provisioned":false})
        );
        assert!(status_summary(&json!({"status":"made-up","provisioned":false})).is_err());
        assert!(status_summary(&json!({"status":"ready"})).is_err());
    }
    #[test]
    fn op5_identity_requires_exact_kind_integer_protocol_and_no_scan() {
        let valid = json!({"wifi_debug":true,"capabilities":"COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1",
            "stage_kind":"private-ram-wifi-debug-stage","debug_protocol":1,"scan":false,
            "debug_generation_limit":8,"provisioned":false});
        assert!(require_debug_identity(&valid).is_ok());
        for key in ["stage_kind", "debug_protocol", "scan"] {
            let mut invalid = valid.clone();
            invalid.as_object_mut().unwrap().remove(key);
            assert!(require_debug_identity(&invalid).is_err());
        }
        for (key, wrong) in [
            ("stage_kind", json!("private-install")),
            ("debug_protocol", json!(true)),
            ("debug_protocol", json!(1.0)),
            ("debug_protocol", json!(2)),
            ("scan", json!(true)),
            ("scan", json!(0)),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = wrong;
            assert!(require_debug_identity(&invalid).is_err());
        }
    }
    #[test]
    fn configuration_rejects_unsafe_or_ambiguous_attachment() {
        let mut value = json!({"schema":1,"bus":1,"ports":[2],"expected_stage":"legacy-status","runtime_root":std::env::temp_dir().join("runtime"),
            "runtime_receipt_sha256":"a".repeat(64)});
        assert!(serde_json::from_value::<Config>(value.clone())
            .unwrap()
            .validate()
            .is_ok());
        let mut missing_receipt = value.clone();
        missing_receipt["expected_stage"] = json!("wifi-debug-v1");
        assert!(serde_json::from_value::<Config>(missing_receipt)
            .unwrap()
            .validate()
            .is_err());
        value["ports"] = json!([]);
        assert!(serde_json::from_value::<Config>(value.clone())
            .unwrap()
            .validate()
            .is_err());
        value["write_boot"] = json!(true);
        assert!(serde_json::from_value::<Config>(value).is_err());
    }
}
