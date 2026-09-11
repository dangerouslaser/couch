//! Opt-in boot-only transition preceding debug attachment; no original capture.
use crate::{
    adapter::{self, Worker},
    dependencies, enrollment,
    frontend::{Choice, Ui},
    public_inputs::create,
    session::SessionGuard,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    source: PathBuf,
    temporary_boot_sha256: String,
    original_boot_sha256: String,
    snapshot_sha256: String,
    image: PathBuf,
    image_sha256: String,
    metadata: PathBuf,
    metadata_sha256: String,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            cfg!(target_os = "linux"),
            "debug boot transitions currently require Linux"
        );
        ensure!(
            [&self.source, &self.image, &self.metadata]
                .iter()
                .all(|p| p.is_absolute()),
            "transition paths must be absolute"
        );
        for pin in [
            &self.temporary_boot_sha256,
            &self.original_boot_sha256,
            &self.snapshot_sha256,
            &self.image_sha256,
            &self.metadata_sha256,
        ] {
            ensure!(
                pin.len() == 64
                    && pin
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "transition requires explicit SHA-256 pins"
            );
        }
        Ok(())
    }
}

fn materialize(session: &SessionGuard) -> Result<PathBuf> {
    let adapter = adapter::materialize(session)?;
    let directory = adapter.parent().context("missing worker directory")?;
    let release = session.path().join("release");
    fs::create_dir(&release)?;
    for (root, name, bytes) in [
        (
            directory,
            "wifi_debug_transition_worker.py",
            include_bytes!("../../wifi_debug_transition_worker.py").as_slice(),
        ),
        (
            directory,
            "wifi_debug_transition.py",
            include_bytes!("../../wifi_debug_transition.py").as_slice(),
        ),
        (
            directory,
            "recover_native_bootstrap.py",
            include_bytes!("../../recover_native_bootstrap.py").as_slice(),
        ),
        (
            directory,
            "enroll_android.py",
            include_bytes!("../../enroll_android.py").as_slice(),
        ),
        (
            directory,
            "capture_readonly.py",
            include_bytes!("../../capture_readonly.py").as_slice(),
        ),
        (
            release.as_path(),
            "prepare_official_inputs.py",
            include_bytes!("../../../release/prepare_official_inputs.py").as_slice(),
        ),
        (
            release.as_path(),
            "official_runtime.py",
            include_bytes!("../../../release/official_runtime.py").as_slice(),
        ),
        (
            release.as_path(),
            "private_vendor.py",
            include_bytes!("../../../release/private_vendor.py").as_slice(),
        ),
        (
            release.as_path(),
            "ha100_official_runtime.json",
            include_bytes!("../../../release/ha100_official_runtime.json").as_slice(),
        ),
    ] {
        let mut file = create(&root.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    Ok(directory.join("wifi_debug_transition_worker.py"))
}

fn rpc(
    worker: &mut Worker,
    ui: &mut Ui,
    operation: &str,
    payload: Value,
    seconds: u64,
) -> Result<Value> {
    worker.operation(Duration::from_secs(seconds), |w| {
        w.send(&json!({"op":operation,"payload":payload}))?;
        // Reuse only the bounded event decoder; never enrollment/capture logic.
        let event = enrollment::event(w, ui, 1)?;
        ensure!(
            event["event"] == operation,
            "unexpected transition response"
        );
        Ok(event["result"].clone())
    })
}

pub fn run(
    ui: &mut Ui,
    session: &mut SessionGuard,
    config: &Config,
    python: &Path,
    runtime: &Path,
    libusb: &Path,
    bus: u8,
    ports: &[u8],
) -> Result<bool> {
    config.validate()?;
    let script = materialize(session)?;
    let mut command = Command::new(python);
    command
        .args(["-I", "-B"])
        .arg(dependencies::python_path(&script)?)
        .arg("--events-stdio")
        .current_dir(session.path());
    let mut worker = Worker::spawn(&mut command)?;
    ensure!(
        rpc(&mut worker, ui, "transition_check", Value::Null, 10)?
            == json!({"device_access":false,"ready":true}),
        "transition worker failed offline check"
    );
    let admitted = rpc(
        &mut worker,
        ui,
        "transition_admit",
        serde_json::to_value(config)?,
        120,
    )?;
    ensure!(
        admitted == json!({"admitted":true,"device_access":false}),
        "invalid offline admission result"
    );
    session.checkpoint(&json!({"event":"debug_transition_inputs_verified","pins":config}))?;
    let choice = ui.choose("Start the dedicated Wi-Fi debug stage", &format!(
        "Verified retained originals and failed-session journal.\nUSB bus {bus}, ports {ports:?}.\nCurrent temporary boot: {}\nDebug boot: {}\nOnly boot will be written and independently verified. Existing originals remain the baseline.",
        config.temporary_boot_sha256, config.image_sha256), &[
        Choice {label:"Cancel".into(),detail:"No USB transition".into()},
        Choice {label:"Perform one debug boot transition".into(),detail:"Wait up to 180 seconds for this remote in preloader mode".into()},
    ])?;
    if choice == 0 {
        rpc(&mut worker, ui, "transition_close", Value::Null, 10)?;
        return Ok(false);
    }
    ui.progress(
        1,
        "Waiting for the selected preloader; then verifying current boot and retained identity",
        0,
        0,
    )?;
    session.checkpoint(&json!({"event":"debug_transition_requested","bus":bus,"ports":ports}))?;
    let result = rpc(
        &mut worker,
        ui,
        "transition_execute",
        json!({"bus":bus,"ports":ports,
        "runtime":dependencies::python_path(runtime)?,"libusb":dependencies::python_path(libusb)?,
        "output":dependencies::python_path(&session.path().join("transition"))?}),
        900,
    )?;
    ensure!(
        result
            == json!({"written":["boot"],"boot_sha256":config.image_sha256,
        "verified":true,"restart_requested":false}),
        "unverified debug transition result"
    );
    session.checkpoint(&json!({"event":"debug_boot_readback_verified","result":result}))?;
    let boot = rpc(&mut worker, ui, "transition_boot", Value::Null, 30)?;
    ensure!(
        boot == json!({"acknowledged":true}),
        "debug boot request was not acknowledged"
    );
    session.checkpoint(&json!({"event":"debug_boot_acknowledged"}))?;
    Ok(true)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn transition_configuration_requires_explicit_pins_and_absolute_paths() {
        let valid = json!({"source":"/retained", "temporary_boot_sha256":"a".repeat(64),
            "original_boot_sha256":"b".repeat(64), "snapshot_sha256":"c".repeat(64),
            "image":"/debug.img", "image_sha256":"d".repeat(64),
            "metadata":"/debug.json", "metadata_sha256":"e".repeat(64)});
        assert_eq!(
            serde_json::from_value::<Config>(valid.clone())
                .unwrap()
                .validate()
                .is_ok(),
            cfg!(target_os = "linux")
        );
        for (key, replacement) in [
            ("source", "relative-session"),
            ("image", "relative.img"),
            ("metadata", "relative.json"),
            ("original_boot_sha256", "ea75"),
            ("temporary_boot_sha256", "222d"),
            ("snapshot_sha256", ""),
            ("image_sha256", "AUTO"),
            ("metadata_sha256", "AUTO"),
        ] {
            let mut value = valid.clone();
            value[key] = json!(replacement);
            assert!(serde_json::from_value::<Config>(value)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut value = valid;
        value["capture_originals"] = json!(true);
        assert!(serde_json::from_value::<Config>(value).is_err());
    }
    #[test]
    fn embedded_transition_worker_imports_and_checks_without_usb() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let session = SessionGuard::create(&root.path().join("run")).unwrap();
        let script = materialize(&session).unwrap();
        let mut command = Command::new("python3");
        command.args(["-I", "-B"]).arg(script).arg("--events-stdio");
        let mut worker = Worker::spawn(&mut command).unwrap();
        worker.operation(Duration::from_secs(10), |w| {
            w.send(&json!({"op":"transition_check","payload":null}))?;
            ensure!(w.event()? == json!({"event":"transition_check","result":{"device_access":false,"ready":true}}));
            w.send(&json!({"op":"transition_close","payload":null}))?;
            ensure!(w.event()?["result"]["closed"] == true);
            Ok(())
        }).unwrap();
    }
}
