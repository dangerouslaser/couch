//! Direct attachment to a RAM Wi-Fi stage. No installer workflow is invoked.
use crate::{
    adapter::{UsbLease, Worker},
    dependencies,
    frontend::{Choice, Ui},
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
    time::Duration,
};

#[derive(Debug, Serialize)]
struct DebugFailure {
    operation: String,
    phase: String,
    category: String,
    errno: Option<i64>,
    backend_error_code: Option<i64>,
}
impl std::fmt::Display for DebugFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Wi-Fi debug {} stopped: {} during {} (errno {:?}, backend {:?}); session preserved",
            self.operation, self.category, self.phase, self.errno, self.backend_error_code
        )
    }
}
impl std::error::Error for DebugFailure {}
impl DebugFailure {
    fn safe(operation: &str, value: &Value) -> Self {
        let operation = match operation {
            "debug_open" | "stage_status" | "debug_status" | "debug_retry" | "debug_close" => {
                operation
            }
            _ => "unknown",
        };
        let phase = match value["phase"].as_str().unwrap_or("") {
            v @ ("request"
            | "attach"
            | "status"
            | "identity"
            | "diagnostic-read"
            | "diagnostic-parse"
            | "diagnostic-contract"
            | "retry"
            | "close"
            | "response") => v,
            _ => "ipc",
        };
        let category = match value["category"].as_str().unwrap_or("") {
            v @ ("USBError" | "USBTimeoutError" | "OSError" | "TimeoutError" | "ContractError"
            | "DecodeError" | "WorkerError") => v,
            _ => "WorkerUnavailable",
        };
        let code = |key: &str| value[key].as_i64().filter(|v| (-65536..=65536).contains(v));
        Self {
            operation: operation.into(),
            phase: phase.into(),
            category: category.into(),
            errno: code("errno"),
            backend_error_code: code("backend_error_code"),
        }
    }
}

// Deliberately separate from the normal installer RPC: debug failures carry
// only reviewed fields, poison the worker, and never trigger retry/reconnect.
fn debug_rpc(worker: &mut Worker, operation: &str, payload: Value) -> Result<Value> {
    worker
        .operation(Duration::from_secs(75), |w| {
            w.send(&json!({"op":operation,"payload":payload}))?;
            let event = w.event()?;
            if event["event"] == "debug_error" {
                ensure!(
                    serde_json::to_vec(&event)?.len() <= 512,
                    "oversized debug error"
                );
                ensure!(
                    event["diagnostic"]["operation"] == operation,
                    "mismatched debug operation"
                );
                return Err(DebugFailure::safe(operation, &event["diagnostic"]).into());
            }
            ensure!(event["event"] == operation, "unexpected debug event");
            Ok(event["result"].clone())
        })
        .map_err(|error| {
            if error.is::<DebugFailure>() {
                error
            } else {
                // Pipe closure, malformed framing, or unknown worker errors must
                // not leak arbitrary bytes through anyhow's error chain.
                DebugFailure::safe(operation, &Value::Null).into()
            }
        })
}

fn stopped_evidence(error: &anyhow::Error) -> Value {
    let mut evidence = json!({"event":"debug_stopped","storage_workflow":false});
    if let Some(failure) = error.downcast_ref::<DebugFailure>() {
        evidence["diagnostic"] = serde_json::to_value(failure).expect("safe debug fields");
    }
    evidence
}

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

/// Retain only the fixed lifecycle record emitted by the dedicated stage.
/// This is an observation, not a host-controlled supervisor interface.
fn lifecycle_summary(value: &Value) -> Result<Value> {
    let lifecycle = value.as_object().context("missing debug lifecycle")?;
    ensure!(
        lifecycle.len() == 8
            && [
                "supervisor",
                "alive",
                "phase",
                "generation",
                "retry",
                "marker",
                "worker",
                "worker_exit",
            ]
            .iter()
            .all(|key| lifecycle.contains_key(*key)),
        "invalid debug lifecycle fields"
    );
    let supervisor = value["supervisor"]
        .as_str()
        .filter(|value| matches!(*value, "couch-wifi-debug-supervisor-v1" | "unknown"));
    let phase = value["phase"].as_str().filter(|value| {
        matches!(
            *value,
            "generation-started"
                | "worker-running"
                | "waiting-retry"
                | "retry-consumed"
                | "retry-limit"
                | "unknown"
        )
    });
    let retry = value["retry"].as_str().filter(|value| {
        matches!(
            *value,
            "none" | "accepted" | "consumed" | "limit" | "unknown"
        )
    });
    let worker = value["worker"]
        .as_str()
        .filter(|value| matches!(*value, "starting" | "running" | "exited" | "unknown"));
    let worker_exit = value["worker_exit"].as_str().filter(|value| {
        matches!(
            *value,
            "none" | "success" | "failure" | "signaled" | "unknown"
        )
    });
    let generation = value["generation"]
        .as_u64()
        .filter(|value| *value <= 8)
        .context("invalid lifecycle generation")?;
    let alive = value["alive"]
        .as_bool()
        .context("invalid lifecycle liveness")?;
    let marker = value["marker"]
        .as_bool()
        .context("invalid lifecycle marker")?;
    Ok(json!({
        "supervisor": supervisor.context("invalid lifecycle supervisor")?,
        "alive": alive,
        "phase": phase.context("invalid lifecycle phase")?,
        "generation": generation,
        "retry": retry.context("invalid lifecycle retry")?,
        "marker": marker,
        "worker": worker.context("invalid lifecycle worker")?,
        "worker_exit": worker_exit.context("invalid lifecycle worker exit")?,
    }))
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
    let lifecycle = lifecycle_summary(&value["lifecycle"])?;
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
        json!({"status":status,"generation":generation,"debug_generation_limit":8,"step":step,"precredential":true,"lifecycle":lifecycle,"log":log}),
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
        debug_rpc(
            &mut worker,
            "debug_open",
            json!({"bus":config.bus,"ports":config.ports,"wait_seconds":if config.transition.is_some() {60} else {0},
            "libusb":dependencies::python_path(&libusb)?}),
        )?;
        loop {
            let raw = debug_rpc(&mut worker, "stage_status", Value::Null)?;
            let status = status_summary(&raw)?;
            session.checkpoint(&json!({"event":"debug_status","result":status}))?;
            if config.expected_stage == "wifi-debug-v1" {
                require_debug_identity(&raw)?;
                let diagnostic =
                    diagnostic_summary(&debug_rpc(&mut worker, "debug_status", Value::Null)?)?;
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
                        "Attempt {} of 8; Wi-Fi {}; step {}; reason {}.\nSupervisor {} ({}, generation {}, marker {}, worker {} / {}).\n{}",
                        diagnostic["generation"],
                        diagnostic["status"]["status"].as_str().unwrap(),
                        diagnostic["step"].as_str().unwrap(),
                        diagnostic["status"]["error"].as_str().unwrap(),
                        diagnostic["lifecycle"]["supervisor"].as_str().unwrap(),
                        if diagnostic["lifecycle"]["alive"].as_bool().unwrap() { "alive" } else { "stale" },
                        diagnostic["lifecycle"]["generation"],
                        diagnostic["lifecycle"]["marker"],
                        diagnostic["lifecycle"]["worker"].as_str().unwrap(),
                        diagnostic["lifecycle"]["worker_exit"].as_str().unwrap(),
                        diagnostic["log"].as_str().unwrap()
                    ),
                    &choices,
                )?;
                if choice == choices.len() - 1 {
                    break;
                }
                if choice == 1 {
                    session.checkpoint(&json!({"event":"wifi_retry_requested","generation":diagnostic["generation"]}))?;
                    debug_rpc(&mut worker, "debug_retry", Value::Null)?;
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
        debug_rpc(&mut worker, "debug_close", Value::Null)?;
        session.checkpoint(&json!({"event":"debug_closed","storage_workflow":false}))?;
        Ok(())
    })();
    if let Err(error) = &result {
        let _ = session.checkpoint(&stopped_evidence(error));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lifecycle() -> Value {
        json!({
            "supervisor": "couch-wifi-debug-supervisor-v1",
            "alive": true,
            "phase": "worker-running",
            "generation": 1,
            "retry": "none",
            "marker": false,
            "worker": "running",
            "worker_exit": "none",
        })
    }

    #[test]
    fn actual_worker_boundary_preserves_safe_failures_and_poisoning() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test_wifi_debug_boundary.py");
        for (case, category, operation) in [
            ("transport", "USBError", "stage_status"),
            ("cleanup", "USBError", "stage_status"),
            ("identity", "ContractError", "debug_status"),
            ("json", "DecodeError", "debug_status"),
            ("valid", "", "debug_status"),
        ] {
            let mut command = Command::new("python3");
            command.args(["-B"]).arg(&script).args(["--fixture", case]);
            let mut worker = Worker::spawn(&mut command).unwrap();
            debug_rpc(
                &mut worker,
                "debug_open",
                json!({"bus":1,"ports":[1],"libusb":"/fake","wait_seconds":0}),
            )
            .unwrap();
            let result = debug_rpc(&mut worker, operation, Value::Null);
            if case == "valid" {
                let value = diagnostic_summary(&result.unwrap()).unwrap();
                assert_eq!(value["generation"], 2);
                assert_eq!(value["status"]["status"], "initializing");
                debug_rpc(&mut worker, "debug_close", Value::Null).unwrap();
            } else {
                let error = result.unwrap_err();
                assert!(error.to_string().contains(category));
                assert!(!error.to_string().contains("SECRET"));
                let evidence = stopped_evidence(&error);
                assert_eq!(evidence["diagnostic"]["operation"], operation);
                assert_eq!(evidence["diagnostic"]["category"], category);
                if category == "USBError" {
                    assert_eq!(evidence["diagnostic"]["errno"], 19);
                    assert_eq!(evidence["diagnostic"]["backend_error_code"], -4);
                }
                // Persist through the real private session writer, not just UI formatting.
                let root = tempfile::tempdir().unwrap();
                let parent = root.path().join("private");
                session::create_private_parent(&parent).unwrap();
                let mut session = SessionGuard::create(&parent.join("boundary")).unwrap();
                session.checkpoint(&evidence).unwrap();
                let saved = std::fs::read(session.path().join("event-00001.json")).unwrap();
                let saved: Value = serde_json::from_slice(&saved).unwrap();
                assert_eq!(saved["evidence"], evidence);
                assert!(!saved.to_string().contains("SECRET"));
                assert!(worker
                    .operation(Duration::from_secs(1), |_| Ok(()))
                    .is_err());
            }
        }
    }

    #[test]
    fn debug_rpc_rejects_untrusted_failure_frames_without_leaking_text() {
        let emit = "import sys,struct; n=struct.unpack('<I',sys.stdin.buffer.read(4))[0]; sys.stdin.buffer.read(n); b=sys.argv[1].encode(); sys.stdout.buffer.write(struct.pack('<I',len(b))+b); sys.stdout.buffer.flush()";
        for frame in [
            json!({"event":"debug_error","diagnostic":{"operation":"debug_open","category":"SECRET","phase":"/private/SECRET","errno":true,"message":"SECRET"}}).to_string(),
            json!({"event":"debug_error","diagnostic":{"operation":"stage_status","category":"USBError"}}).to_string(),
            json!({"event":"debug_error","diagnostic":{"operation":"debug_open","category":"USBError","extra":"SECRET".repeat(200)}}).to_string(),
            "{SECRET".into(),
        ] {
            let mut command = Command::new("python3");
            command.args(["-B", "-c", emit, &frame]);
            let mut worker = Worker::spawn(&mut command).unwrap();
            let error = debug_rpc(&mut worker, "debug_open", Value::Null).unwrap_err();
            assert!(error.to_string().contains("WorkerUnavailable"));
            assert!(!error.to_string().contains("SECRET"));
            assert!(!stopped_evidence(&error).to_string().contains("SECRET"));
            assert!(worker.operation(Duration::from_secs(1), |_| Ok(())).is_err());
        }
    }

    #[test]
    fn failure_display_and_evidence_are_bounded_and_allowlisted() {
        let error = DebugFailure::safe(
            "stage_status",
            &json!({
            "operation":"SECRET", "phase":"/private/SECRET", "category":"SECRET".repeat(1000),
            "errno":true,"backend_error_code":65537,"message":"SECRET"}),
        );
        assert_eq!(error.operation, "stage_status");
        assert_eq!(error.phase, "ipc");
        assert_eq!(error.category, "WorkerUnavailable");
        let text = error.to_string();
        assert!(!text.contains("SECRET") && text.len() < 256);
        assert!(error.errno.is_none() && error.backend_error_code.is_none());
    }
    #[test]
    fn debug_identity_and_diagnostics_are_required_before_actions() {
        assert!(require_debug_identity(&json!({"status":"ready","provisioned":false})).is_err());
        let mut status = json!({"status":"failed","provisioned":false,"wifi_debug":true,
            "stage_kind":"private-ram-wifi-debug-stage","debug_protocol":1,"scan":false,
            "capabilities":"COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1", "debug_generation_limit":8});
        assert!(require_debug_identity(&status).is_ok());
        let mut diagnostic = json!({"status":status,"generation":1,"step":"power",
            "stage_kind":"private-ram-wifi-debug-stage", "capability":"COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1", "debug_protocol":1,
            "precredential":true,"debug_generation_limit":8,"lifecycle":lifecycle(),"log":"message\u{001b}\u{202e}\nnext"});
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
            "lifecycle":lifecycle(),
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
    fn lifecycle_is_retained_only_when_typed_and_complete() {
        let valid = lifecycle();
        assert_eq!(lifecycle_summary(&valid).unwrap(), valid);
        for key in [
            "supervisor",
            "alive",
            "phase",
            "generation",
            "retry",
            "marker",
            "worker",
            "worker_exit",
        ] {
            let mut invalid = valid.clone();
            invalid.as_object_mut().unwrap().remove(key);
            assert!(lifecycle_summary(&invalid).is_err());
        }
        for (key, value) in [
            ("alive", json!(1)),
            ("phase", json!("credentials")),
            ("generation", json!(9)),
            ("retry", json!("pending")),
            ("worker", json!("pid-123")),
            ("worker_exit", json!("secret")),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(lifecycle_summary(&invalid).is_err());
        }
        let mut extra = valid.clone();
        extra["private"] = json!("text");
        assert!(lifecycle_summary(&extra).is_err());
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
