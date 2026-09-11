//! Migration of retained Python evidence without manufacturing a native journal.
use super::*;
use std::io::{Seek, SeekFrom};

/// `expected_stock_manifest_sha256` must come from a separately trusted retained
/// package/record, never be inferred from this source directory during import.
/// All originals present in the old backup set, including userdata, are retained.
pub fn import_legacy(
    source: &Path,
    stock_manifest: &Path,
    expected_stock_manifest_sha256: &str,
    destination: &mut SessionGuard,
    mut progress: impl FnMut(&str, u64, u64) -> Result<()>,
) -> Result<SavedEnrollment> {
    ensure!(
        matches!(destination.phase(), Phase::Created | Phase::InputsVerified),
        "legacy import must precede live binding"
    );
    ensure!(
        fs::symlink_metadata(source)?.is_dir() && valid_hex(expected_stock_manifest_sha256, 64),
        "invalid retained enrollment source or independent profile pin"
    );
    let profile_bytes = read(stock_manifest, 65536)?;
    ensure!(
        digest(&profile_bytes) == expected_stock_manifest_sha256,
        "retained stock manifest differs from independent trusted pin"
    );
    let profile: Value = serde_json::from_slice(&profile_bytes)?;
    ensure!(
        profile["schema"] == 1
            && profile["model"] == "sanytron-ha100"
            && profile["purpose"] == "stock-baseline"
            && profile["private_candidate"] == true
            && profile["installable"] == false,
        "not a reviewed retained stock profile"
    );
    let mut evidence = BTreeMap::new();
    for name in [
        "bootstrap/baseline.json",
        "bootstrap/journal.json",
        "bootstrap/backup-receipt.json",
        "private-image/plan.json",
        "originals/journal.json",
    ] {
        evidence.insert(name.to_string(), read(&source.join(name), 1024 * 1024)?);
    }
    let parse = |name: &str| -> Result<Value> { Ok(serde_json::from_slice(&evidence[name])?) };
    let baseline = parse("bootstrap/baseline.json")?;
    let bootstrap = parse("bootstrap/journal.json")?;
    let receipt = parse("bootstrap/backup-receipt.json")?;
    let plan = parse("private-image/plan.json")?;
    let journal = parse("originals/journal.json")?;
    ensure!(
        receipt["schema"] == 1
            && receipt["complete"] == true
            && receipt["journal_sha256"] == digest(&evidence["bootstrap/journal.json"]),
        "legacy bootstrap receipt changed"
    );
    ensure!(
        bootstrap["schema"] == 1
            && bootstrap["purpose"] == "private-stock-to-ram-bootstrap"
            && bootstrap["complete"] == true
            && bootstrap["backup_complete"] == true
            && bootstrap["usb_cleanup_verified"] == true
            && bootstrap["identity_verified_after"] == true
            && bootstrap["retained_verified_after"] == true
            && bootstrap["write"] == "verified"
            && bootstrap["stock_manifest_sha256"] == expected_stock_manifest_sha256,
        "incomplete or untrusted legacy bootstrap evidence"
    );
    ensure!(
        bootstrap.get("original_os").is_none_or(|v| v == "Android")
            && journal.get("original_os").is_none_or(|v| v == "Android"),
        "retained backups contain Couch, not Android originals"
    );
    ensure!(
        journal["schema"] == 1
            && journal["backup_complete"] == true
            && journal["bootstrap_receipt_sha256"]
                == digest(&evidence["bootstrap/backup-receipt.json"])
            && journal["plan_sha256"] == digest(&serde_json::to_vec(&plan)?),
        "legacy transaction backup proof changed"
    );
    ensure!(
        plan["schema"] == 1
            && journal["nonce"] == plan["nonce"]
            && bootstrap["stage_sha256"] == plan["stage_sha256"],
        "legacy plan binding differs"
    );
    for key in ["cid", "capacity", "partitions", "identity_sha256"] {
        ensure!(
            baseline[key] == plan[key],
            "legacy baseline differs from bound plan"
        );
    }
    ensure!(
        baseline["partitions"] == profile["partitions"]
            && baseline["partitions"] == bootstrap["device"]["partitions"]
            && baseline["capacity"] == bootstrap["device"]["capacity"]
            && baseline["identity_sha256"] == bootstrap["identity_sha256"]
            && baseline["identity_sha256"] == receipt["identity_sha256"],
        "legacy device or calibration evidence differs"
    );
    let cid = baseline["cid"].as_str().context("legacy CID absent")?;
    ensure!(valid_hex(cid, 32), "invalid legacy canonical CID");
    let raw = (0..16)
        .map(|i| u8::from_str_radix(&cid[i * 2..i * 2 + 2], 16))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        bootstrap["device"]["hwcode"] == 0x6580
            && bootstrap["device"]["cid_encoding"] == "mt6580-legacy-le32-registers"
            && bootstrap["device"]["runtime_cid_sha256"] == digest(&raw)
            && receipt["cid_sha256"] == digest(&raw),
        "legacy storage identity binding differs"
    );
    let hashes: BTreeMap<String, String> = serde_json::from_value(journal["backups"].clone())?;
    for key in [
        "nonce",
        "manifest_sha256",
        "stage_sha256",
        "original_boot_sha256",
    ] {
        ensure!(
            plan[key].as_str().is_some_and(|v| valid_hex(v, 64)),
            "invalid legacy plan digest"
        );
    }
    let mut expected_names = plan["images"]
        .as_object()
        .context("missing legacy image inventory")?
        .keys()
        .map(String::as_str)
        .chain(IDENTITY)
        .collect::<BTreeSet<_>>();
    if plan.get("skip_userdata_backup").is_some_and(|v| v == true) {
        expected_names.remove("userdata");
    }
    ensure!(
        hashes.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected_names,
        "legacy original backup inventory incomplete"
    );

    for name in ["boot", "recovery"] {
        ensure!(
            hashes
                .get(name)
                .is_some_and(|h| bootstrap["originals"][name] == *h
                    && receipt["originals"][name] == *h
                    && bootstrap["stock_hashes"][name] == *h),
            "legacy original bootstrap image differs"
        );
    }
    if let Some(recovery) = profile["images"].get("recovery") {
        ensure!(
            recovery["sha256"]
                .as_str()
                .is_some_and(|v| valid_hex(v, 64))
                && journal["backups"]["recovery"] == recovery["sha256"],
            "saved recovery differs from independently trusted profile"
        );
    }
    ensure!(
        journal["backups"]["boot"] == plan["original_boot_sha256"]
            && journal["backups"]["boot"] == profile["images"]["boot"]["sha256"],
        "saved boot does not establish retained Android stock profile"
    );
    let odmdtbo = profile["images"]["odmdtbo"]["sha256"]
        .as_str()
        .context("retained overlay evidence absent")?;
    ensure!(
        valid_hex(odmdtbo, 64) && bootstrap["stock_hashes"]["odmdtbo"] == odmdtbo,
        "retained overlay differs from trusted stock profile"
    );
    let partitions: BTreeMap<String, Region> =
        serde_json::from_value(baseline["partitions"].clone())?;
    let mut originals = BTreeMap::new();
    for (name, sha256) in hashes {
        let region = partitions
            .get(&name)
            .context("unknown original partition")?;
        originals.insert(
            name.clone(),
            Original {
                file: format!("bootstrap-{name}.img"),
                size: region.size,
                sha256,
            },
        );
    }
    let record = Record {
        schema: 1,
        kind: "couch-retained-python-enrollment".into(),
        model: "sanytron-ha100".into(),
        cid: cid.into(),
        capacity: baseline["capacity"].as_u64().context("capacity absent")?,
        partitions,
        identity_sha256: serde_json::from_value(baseline["identity_sha256"].clone())?,
        android_identity: None,
        original_os: "Android".into(),
        originals,
    };
    validate_record(&record, true)?;
    let mut admitted = Vec::new();
    for (name, entry) in &record.originals {
        progress(name, 0, entry.size)?;
        let mut file = crate::regular(&source.join("originals").join(format!("{name}.img")))?;
        ensure!(
            file.metadata()?.len() == entry.size,
            "legacy original size differs"
        );
        verify_file(&mut file, entry, &mut progress, name)?;
        admitted.push((name.clone(), entry.clone(), file));
    }
    let root = destination.path().join("imported-enrollment");
    crate::private_dir(&root)?;
    for (name, entry, mut source) in admitted {
        source.seek(SeekFrom::Start(0))?;
        let mut output = create(&root.join(&entry.file))?;
        let mut remaining = entry.size;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 1024 * 1024];
        while remaining > 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            source.read_exact(&mut buffer[..count])?;
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
            remaining -= count as u64;
            progress(&name, entry.size - remaining, entry.size)?;
        }
        ensure!(
            source.read(&mut [0; 1])? == 0 && format!("{:x}", hash.finalize()) == entry.sha256,
            "legacy original changed during private copy"
        );
        output.sync_all()?;
    }
    // These are copies of original Python evidence, not new native capture events.
    for (name, bytes) in evidence.iter().chain(std::iter::once((
        &"stock-manifest.json".to_string(),
        &profile_bytes,
    ))) {
        let filename = name.replace('/', "-");
        let mut file = create(&root.join(filename))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    let normalized = serde_json::to_vec(&record)?;
    let checksum = digest(&normalized);
    let mut file = create(&root.join("imported-legacy-baseline.json"))?;
    file.write_all(&normalized)?;
    file.sync_all()?;
    drop(file);
    crate::session::sync_private_directory(&root)?;
    destination.checkpoint(&json!({"event":"legacy_android_enrollment_imported","enrollment_sha256":checksum,"stock_manifest_sha256":expected_stock_manifest_sha256,"source_receipts_sha256":evidence.iter().map(|(name,bytes)|(name.clone(),digest(bytes))).collect::<BTreeMap<_,_>>(),"android_identity_known":false,"live_hardware_rebound":false}))?;
    Ok(SavedEnrollment {
        record,
        root,
        sha256: checksum,
        retained_sha256: BTreeMap::from([("odmdtbo".into(), odmdtbo.into())]),
        provenance: "retained-python-android-bootstrap",
    })
}
fn create(path: &Path) -> Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}
fn verify_file(
    file: &mut std::fs::File,
    entry: &Original,
    progress: &mut impl FnMut(&str, u64, u64) -> Result<()>,
    name: &str,
) -> Result<()> {
    let mut remaining = entry.size;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    while remaining > 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..count])?;
        hash.update(&buffer[..count]);
        remaining -= count as u64;
        progress(name, entry.size - remaining, entry.size)?;
    }
    ensure!(
        file.read(&mut [0; 1])? == 0 && format!("{:x}", hash.finalize()) == entry.sha256,
        "legacy original hash differs"
    );
    Ok(())
}
