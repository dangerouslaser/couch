//! Native fresh-Android installer. Every mutation follows durable enrollment;
//! the Python worker is restricted to MTK and bounded USB setup transport.
use crate::{
    adapter::{self, UsbLease, Worker},
    android, assembly, dependencies, enrollment,
    frontend::{Choice, Ui},
    network,
    public_inputs::{self, create, decode, digest, hex},
    session::{self, Phase, SessionGuard},
    stage::Channel,
    stage_tls,
    transaction::{self, OriginalOs},
    vendor_transfer,
};
use anyhow::{ensure, Context, Result};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
fn random() -> Result<[u8; 32]> {
    let mut value = [0; 32];
    SystemRandom::new()
        .fill(&mut value)
        .map_err(|_| anyhow::anyhow!("secure randomness unavailable"))?;
    Ok(value)
}
fn choice(label: &str, detail: &str) -> Choice {
    Choice {
        label: label.into(),
        detail: detail.into(),
    }
}
fn state_root() -> Result<PathBuf> {
    #[cfg(windows)]
    let path = PathBuf::from(
        std::env::var_os("LOCALAPPDATA").context("missing local application data directory")?,
    )
    .join("CouchInstaller");
    #[cfg(not(windows))]
    let path = PathBuf::from(std::env::var_os("HOME").context("missing home directory")?)
        .join(".couch-installer");
    if !path.exists() {
        session::create_private_parent(&path)?;
    }
    Ok(path)
}
fn simple(
    worker: &mut Worker,
    command: Value,
    event: &str,
    budget: u64,
    ui: &mut Ui,
    phase: usize,
) -> Result<Value> {
    worker.operation(Duration::from_secs(budget), |w| {
        w.send(&command)?;
        let result = enrollment::event(w, ui, phase)?;
        ensure!(result["event"] == event, "unexpected USB operation result");
        Ok(result)
    })
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    ensure!(
        digest(path)? == format!("{:x}", Sha256::digest(bytes)),
        "private artifact readback differs"
    );
    Ok(())
}
fn identity_input(
    ui: &mut Ui,
    label: &str,
    available: Option<String>,
    mac: bool,
) -> Result<String> {
    if let Some(value) = available {
        return Ok(value);
    }
    loop {
        let value = ui
            .input(
                label,
                "Read this value from Android device information before the remote restarts.",
                false,
            )?
            .to_string();
        let valid = if mac {
            let bytes = decode(&value.to_ascii_lowercase().replace(':', ""));
            !value.eq_ignore_ascii_case("02:00:00:00:00:00")
                && value.len() == 17
                && value.as_bytes().iter().enumerate().all(|(i, c)| {
                    if i % 3 == 2 {
                        *c == b':'
                    } else {
                        c.is_ascii_hexdigit()
                    }
                })
                && bytes.is_ok_and(|b| {
                    b.len() == 6
                        && b[0] & 1 == 0
                        && b.iter().any(|c| *c != 0)
                        && b.iter().any(|c| *c != 255)
                })
        } else {
            (8..=64).contains(&value.len()) && value.bytes().all(|c| c.is_ascii_alphanumeric())
        };
        if valid {
            return Ok(if mac {
                value.to_ascii_lowercase()
            } else {
                value
            });
        }
        ui.progress(
            1,
            "That device-information value is not valid. Check Android and try again.",
            0,
            0,
        )?;
    }
}
fn plan_images(paths: &BTreeMap<String, PathBuf>, device: &Value, ui: &mut Ui) -> Result<Value> {
    let mut images = serde_json::Map::new();
    for (name, path) in paths {
        let size = fs::metadata(path)?.len();
        ensure!(
            size > 0
                && size.is_multiple_of(512)
                && size
                    <= device["partitions"][name]["size"]
                        .as_u64()
                        .context("missing target partition")?,
            "assembled image does not fit"
        );
        let mut file = fs::File::open(path)?;
        let mut chunks = Vec::new();
        let mut full = Sha256::new();
        let mut buffer = vec![0; crate::stage::CHUNK];
        let mut done = 0;
        while done < size {
            let n = (size - done).min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..n])?;
            full.update(&buffer[..n]);
            chunks.push(format!("{:x}", Sha256::digest(&buffer[..n])));
            done += n as u64;
            ui.progress(4, &format!("Verifying {name} image"), done, size)?;
        }
        ensure!(
            file.read(&mut [0u8; 1])? == 0 && file.metadata()?.len() == size,
            "image changed during hashing"
        );
        images.insert(
            name.clone(),
            json!({"size":size,"sha256":format!("{:x}",full.finalize()),"chunks":chunks}),
        );
    }
    Ok(Value::Object(images))
}

pub fn run(ui: &mut Ui, config: Option<&Path>) -> Result<()> {
    ui.set_steps(
        [
            "Prepare",
            "Android",
            "Originals",
            "Start installer",
            "Wi-Fi",
            "Backups",
            "Install",
            "Finish",
        ]
        .map(String::from)
        .to_vec(),
    )?;
    let mode = ui.choose(
        "Install Couch",
        "Start with Android running. Enable USB debugging and connect your remote via USB.",
        &[
            choice(
                "Install with Android backup",
                "Preserve Android data and device calibration before installing.",
            ),
            choice(
                "YOLO — skip Android data backup",
                "Device calibration and recovery originals are still preserved.",
            ),
            choice("Cancel", "Leave the device unchanged."),
        ],
    )?;
    if mode == 2 {
        return Ok(());
    }
    let release = public_inputs::release(
        config.context("This installer requires its verified release configuration")?,
    )?;
    let parent = state_root()?;
    let mut session =
        SessionGuard::create(&parent.join(format!("install-{}", &hex(&random()?)[..16])))?;
    ui.set_log_path(
        session
            .path()
            .to_str()
            .context("invalid private session path")?,
    )?;
    let result = install(ui, &release, mode == 1, &mut session);
    if let Err(error) = &result {
        if !matches!(session.phase(), Phase::Failed | Phase::Complete) {
            let _ = session.transition(
                Phase::Failed,
                &json!({"event":"installation_stopped","preserve_originals":true}),
            );
        }
        let _ = ui.error(&format!(
            "{error}. Keep saved originals at {}. No automatic retry or restore was attempted.",
            session.path().display()
        ));
    }
    result
}
fn install(
    ui: &mut Ui,
    release: &public_inputs::Release,
    skip_userdata: bool,
    session: &mut SessionGuard,
) -> Result<()> {
    let _usb_lease = UsbLease::acquire(session)?;
    let dependencies = dependencies::prepare(
        session,
        dependencies::host_platform()?,
        |label, done, total| ui.progress(0, label, done, total),
    )?;
    let ota = session.path().join("official-ota.zip");
    public_inputs::official(&ota, |done, total| {
        ui.progress(0, "Downloading verified official owner inputs", done, total)
    })?;
    let prepared = session.path().join("owner-inputs");
    ui.progress(0, "Reconstructing verified owner inputs", 0, 0)?;
    crate::prepare(&ota, &prepared)?;
    let public = public_inputs::payload(release, session.path(), |done, total| {
        ui.progress(0, "Downloading Couch OS", done, total)
    })?;
    let mut images = BTreeMap::new();
    for (name, ramdisk) in [
        ("boot", "boot.cpio.gz"),
        ("recovery", "recovery.cpio.gz"),
        ("installer", "installer.cpio.gz"),
    ] {
        let ram = assembly::owner_ramdisk(&fs::read(&public[ramdisk])?, &prepared)?;
        let kernel = fs::read(&public["zImage"])?;
        let bytes = assembly::boot_image(
            &prepared,
            if name == "recovery" {
                None
            } else {
                Some(&kernel)
            },
            &ram,
        )?;
        let path = session.path().join(format!("{name}.img"));
        write(&path, &bytes)?;
        images.insert(name.to_string(), path);
    }
    let stage = images.remove("installer").unwrap();
    let stage_hash = digest(&stage)?;
    images.insert("userdata".into(), public["userdata.ext4"].clone());
    let vendor = vendor_transfer::prepare(&prepared)?;
    session.transition(Phase::InputsVerified,&json!({"event":"inputs_verified","release":release.version,"payload_sha256":release.payload.sha256,"stage_sha256":stage_hash}))?;
    ui.progress(
        1,
        "Enable USB debugging, connect the remote, and accept Android's USB authorization prompt.",
        0,
        0,
    )?;
    dependencies.verify()?;
    let devices = android::discover(&dependencies.adb)?;
    ensure!(
        !devices.is_empty(),
        "No Android remote found. Enable USB debugging and connect USB"
    );
    let selected = ui.choose(
        "Choose your Android remote",
        "Only USB-connected Android devices are listed.",
        &devices
            .iter()
            .map(|d| {
                choice(
                    &d.serial,
                    &format!("{} · {}", d.model.as_deref().unwrap_or("Android"), d.state),
                )
            })
            .collect::<Vec<_>>(),
    )?;
    let android = android::capture(&dependencies.adb, &devices[selected].serial)?;
    let cid = android
        .cid
        .as_deref()
        .context("Android did not expose a usable eMMC identity; no write is permitted")?;
    let identity = json!({"device_id":identity_input(ui,"Android Device ID",android.device_id,false)?,"wifi_mac":identity_input(ui,"Android Wi-Fi MAC",android.wifi_mac,true)?,"bt_mac":identity_input(ui,"Android Bluetooth MAC",android.bt_mac,true)?});
    dependencies.verify()?;
    let script = adapter::materialize(session)?;
    let mut command = Command::new(&dependencies.python);
    command
        .arg("-I")
        .arg("-B")
        .arg(dependencies::python_path(&script)?)
        .arg("--events-stdio")
        .current_dir(session.path());
    let mut worker = Worker::spawn(&mut command)?;
    simple(
        &mut worker,
        json!({"op":"prepare","checkout":dependencies::python_path(&dependencies.mtk_root)?,"loader":dependencies::python_path(&dependencies.owner_da)?,"loader_sha256":dependencies.owner_da_sha256,"preloader":dependencies::python_path(&prepared.join("bootstrap/preloader.img"))?,"preloader_sha256":"0ad0d14b7203d98a6567af7a022cfe5df5b6fcbba60cb4e9b4bc2ee569cf1069","libusb":dependencies::python_path(&dependencies.libusb)?}),
        "prepared",
        60,
        ui,
        1,
    )?;
    let bound = simple(
        &mut worker,
        json!({"op":"android_bind","serial":android.serial}),
        "android_bound",
        30,
        ui,
        1,
    )?;
    ensure!(
        bound["serial_sha256"] == format!("{:x}", Sha256::digest(android.serial.as_bytes())),
        "Android USB serial binding differs"
    );
    session.transition(
        Phase::AndroidBound,
        &json!({"event":"android_bound","cid":cid,"usb":bound,"android_identity":identity}),
    )?;
    ui.progress(3, "Restarting the selected remote through USB", 0, 0)?;
    dependencies.verify()?;
    android::reboot(&dependencies.adb, &android.serial)?;
    let start = Instant::now();
    let candidate = loop {
        ensure!(start.elapsed()<Duration::from_secs(120),"Remote did not enter download mode. No boot write occurred; check USB driver binding and power");
        let result = simple(
            &mut worker,
            json!({"op":"enumerate"}),
            "candidates",
            20,
            ui,
            3,
        )?;
        let found = result["devices"]
            .as_array()
            .context("invalid USB inventory")?
            .iter()
            .filter(|d| d["bus"] == bound["bus"] && d["ports"] == bound["ports"])
            .collect::<Vec<_>>();
        ensure!(found.len() <= 1, "ambiguous selected USB port");
        if let Some(device) = found.first() {
            break (*device).clone();
        }
        ui.progress(
            3,
            "Waiting for the selected remote on USB",
            start.elapsed().as_secs(),
            120,
        )?;
        thread::sleep(Duration::from_millis(100));
    };
    let connected=simple(&mut worker,json!({"op":"start","candidate":candidate}),"connected",120,ui,3).context("Could not claim the selected USB download interface. On Windows it needs a compatible WinUSB driver; do not replace drivers for other USB devices")?;
    let device = &connected["device"];
    enrollment::admit_layout(device, cid, &prepared)?;
    let originals = enrollment::capture(&mut worker, device, session, ui)?;
    enrollment::stock_prefixes(session, &prepared)?;
    let logo = assembly::logo_image(
        &fs::read(session.path().join("bootstrap-logo.img"))?,
        &fs::read(
            public
                .get("logo.bgra")
                .context("public Couch logo frame missing")?,
        )?,
    )?;
    let logo_path = session.path().join("logo.img");
    write(&logo_path, &logo)?;
    images.insert("logo".into(), logo_path);
    let identity_hashes: BTreeMap<_, _> = enrollment::IDENTITY
        .into_iter()
        .map(|n| (n, originals[n].clone()))
        .collect();
    let original_hashes: BTreeMap<_, _> = enrollment::ORIGINALS
        .into_iter()
        .map(|n| (n, originals[n].clone()))
        .collect();
    let entries:BTreeMap<_,_>=originals.iter().map(|(name,sha)|(name,json!({"file":format!("bootstrap-{name}.img"),"size":device["partitions"][name]["size"],"sha256":sha}))).collect();
    let enrollment_path = session.path().join("enrollment.json");
    write(
        &enrollment_path,
        &serde_json::to_vec(
            &json!({"schema":1,"kind":"couch-device-enrollment","model":"sanytron-ha100","cid":cid,"capacity":device["capacity"],"partitions":device["partitions"],"identity_sha256":identity_hashes,"android_identity":identity,"original_os":"Android","originals":entries}),
        )?,
    )?;
    session.transition(
        Phase::OriginalsSaved,
        &json!({"event":"enrollment_complete","enrollment_sha256":digest(&enrollment_path)?}),
    )?;
    simple(
        &mut worker,
        json!({"op":"authorize_boot","cid_sha256":device["runtime_cid_sha256"],"partitions":device["partitions"],"identity_sha256":identity_hashes,"original_sha256":original_hashes,"stage":dependencies::python_path(&stage)?,"stage_sha256":stage_hash}),
        "boot_authorized",
        1800,
        ui,
        2,
    )?;
    session.transition(
        Phase::StageBootPending,
        &json!({"event":"bootstrap_write_admitted","stage_sha256":stage_hash}),
    )?;
    simple(
        &mut worker,
        json!({"op":"write_boot"}),
        "boot_verified",
        1800,
        ui,
        3,
    )?;
    session
        .checkpoint(&json!({"event":"bootstrap_readback_verified","stage_sha256":stage_hash}))?;
    simple(
        &mut worker,
        json!({"op":"boot"}),
        "boot_requested",
        30,
        ui,
        3,
    )?;
    let start = Instant::now();
    loop {
        ensure!(
            start.elapsed() < Duration::from_secs(120),
            "Installer stage did not appear. Keep the saved originals"
        );
        if simple(
            &mut worker,
            json!({"op":"stage_present"}),
            "stage_present",
            20,
            ui,
            3,
        )?["present"]
            == true
        {
            break;
        }
        ui.progress(
            3,
            "Waiting for the installer USB stage",
            start.elapsed().as_secs(),
            120,
        )?;
        thread::sleep(Duration::from_millis(250));
    }
    simple(
        &mut worker,
        json!({"op":"stage_open"}),
        "stage_open",
        60,
        ui,
        3,
    )?;
    network::ready(&mut worker, ui)?;
    let network = network::select(&mut worker, ui)?;
    let plan = json!({"schema":1,"nonce":hex(&random()?),"manifest_sha256":release.payload.sha256,"stage_sha256":stage_hash,"original_boot_sha256":originals["boot"],"cid":cid,"capacity":device["capacity"],"partitions":device["partitions"],"identity_sha256":identity_hashes,"images":plan_images(&images,device,ui)?,"skip_userdata_backup":skip_userdata,"network":network,"vendor_source_sha256":vendor.source_sha256()});
    let plan_bytes = zeroize::Zeroizing::new(serde_json::to_vec(&plan)?);
    network::rpc(
        &mut worker,
        "stage_bind",
        json!({"plan_sha256":format!("{:x}",Sha256::digest(plan_bytes.as_slice())),"nonce":plan["nonce"]}),
    )?;
    let cert = rcgen::generate_simple_self_signed(vec!["couch-probe".into()])?;
    let der = cert.cert.der().to_vec();
    let key = zeroize::Zeroizing::new(cert.signing_key.serialize_der());
    let token = zeroize::Zeroizing::new(random()?);
    let mut provision = network;
    let object = provision.as_object_mut().unwrap();
    object.insert("certificate_hex".into(), json!(hex(&der)));
    object.insert("private_key_hex".into(), json!(hex(&key)));
    object.insert("token_hex".into(), json!(hex(token.as_ref())));
    network::rpc(&mut worker, "stage_provision", provision)?;
    let start = Instant::now();
    let address = loop {
        ensure!(
            start.elapsed() < Duration::from_secs(120),
            "Remote Wi-Fi connection timed out"
        );
        let status = network::rpc(&mut worker, "stage_status", Value::Null)?;
        ensure!(
            status["status"] != "failed",
            "Remote Wi-Fi connection failed; preserve originals"
        );
        if status["status"] == "connected" {
            break status["ip"]
                .as_str()
                .context("missing Wi-Fi address")?
                .parse::<std::net::Ipv4Addr>()?;
        }
        ui.progress(
            4,
            "Connecting the remote to Wi-Fi",
            start.elapsed().as_secs(),
            120,
        )?;
        thread::sleep(Duration::from_millis(500));
    };
    simple(
        &mut worker,
        json!({"op":"stage_close"}),
        "stage_closed",
        30,
        ui,
        4,
    )?;
    drop(worker);
    let mut connection = stage_tls::connect((address, 8443).into(), &der, &token)?;
    connection
        .sock
        .set_phase_timeout(Duration::from_secs(1800))?;
    session.transition(Phase::StageConnected,&json!({"event":"stage_authenticated","plan_sha256":format!("{:x}",Sha256::digest(plan_bytes.as_slice()))}))?;
    let mut channel = Channel::authenticated(connection);
    transaction::run_with_vendor(
        &mut channel,
        &plan,
        &images,
        &enrollment::original_boot(session),
        OriginalOs::Android,
        session,
        Some(vendor),
        |phase, target, done, total| {
            ui.progress(
                if phase.contains("Backup") { 5 } else { 6 },
                &format!("{phase} {target}"),
                done,
                total,
            )
        },
    )?;
    let reboot = ui.choose(
        "Couch installation verified",
        "Backups and the installation journal are saved on this computer.",
        &[
            choice("Restart into Couch", "Start the newly installed OS."),
            choice(
                "Leave the installer running",
                "Keep the current verified stage available.",
            ),
        ],
    )?;
    channel.send_json(&json!({"action":if reboot==0{"reboot"}else{"leave"}}))?;
    if reboot == 0 {
        channel.expect(&json!({"event":"rebooting"}))?;
    }
    ui.progress(
        7,
        "Installation verified. First normal boot still needs to be checked on the remote.",
        1,
        1,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn cancel_never_requires_release_config_or_creates_session() {
        let mut ui = Ui::new(
            Box::new(Cursor::new(b"{\"id\":1,\"value\":\"2\"}\n".to_vec())),
            Box::new(Vec::new()),
        );
        run(&mut ui, None).unwrap();
    }
    #[test]
    fn images_use_fixed_wire_chunks_and_full_digest() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("image");
        let data = vec![42; crate::stage::CHUNK + 512];
        fs::write(&path, &data).unwrap();
        let paths = BTreeMap::from([("userdata".into(), path)]);
        let device = json!({"partitions":{"userdata":{"size":data.len()}}});
        let mut ui = Ui::new(Box::new(Cursor::new(Vec::new())), Box::new(Vec::new()));
        ui.set_steps(vec!["step".into(); 8]).unwrap();
        let result = plan_images(&paths, &device, &mut ui).unwrap();
        assert_eq!(
            result["userdata"]["chunks"],
            json!([
                format!("{:x}", Sha256::digest(&data[..crate::stage::CHUNK])),
                format!("{:x}", Sha256::digest(&data[crate::stage::CHUNK..]))
            ])
        );
        assert_eq!(
            result["userdata"]["sha256"],
            format!("{:x}", Sha256::digest(&data))
        );
    }
}
