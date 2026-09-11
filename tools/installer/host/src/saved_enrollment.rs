//! Read-only admission of retained Android enrollment. Import never authorizes writes.
mod legacy;
use crate::session::{Phase, SessionGuard};
use anyhow::{ensure, Context, Result};
pub use legacy::import_legacy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const IDENTITY: [&str; 5] = ["proinfo", "nvram", "nvdata", "protect1", "protect2"];
const ORIGINALS: [&str; 9] = [
    "proinfo", "nvram", "nvdata", "protect1", "protect2", "boot", "recovery", "odmdtbo", "logo",
];
const STARTS: [(&str, u64); 23] = [
    ("proinfo", 0x80000),
    ("nvram", 0x380000),
    ("protect1", 0x880000),
    ("protect2", 0x1280000),
    ("seccfg", 0x1c80000),
    ("lk", 0x1cc0000),
    ("lk2", 0x1d20000),
    ("boot", 0x1d80000),
    ("recovery", 0x2d80000),
    ("para", 0x3d80000),
    ("logo", 0x3e00000),
    ("odmdtbo", 0x4600000),
    ("expdb", 0x5600000),
    ("vendor", 0x6000000),
    ("frp", 0x18000000),
    ("nvdata", 0x18100000),
    ("metadata", 0x1a100000),
    ("oemkeystore", 0x1c800000),
    ("secro", 0x1ca00000),
    ("keystore", 0x1d000000),
    ("system", 0x1d800000),
    ("cache", 0x6a000000),
    ("userdata", 0x71000000),
];
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub offset: u64,
    pub size: u64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Original {
    pub file: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AndroidIdentity {
    pub device_id: String,
    pub wifi_mac: String,
    pub bt_mac: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: u32,
    pub kind: String,
    pub model: String,
    pub cid: String,
    pub capacity: u64,
    pub partitions: BTreeMap<String, Region>,
    pub identity_sha256: BTreeMap<String, String>,
    pub android_identity: Option<AndroidIdentity>,
    pub original_os: String,
    pub originals: BTreeMap<String, Original>,
}
pub struct SavedEnrollment {
    record: Record,
    root: PathBuf,
    sha256: String,
    retained_sha256: BTreeMap<String, String>,
    provenance: &'static str,
}
/// Constructed from fresh observations through the selected physical USB session,
/// never by copying fields from the imported record or TUI input.
pub struct ObservedHardware {
    pub cid: String,
    pub capacity: u64,
    pub hwcode: u32,
    pub cid_encoding: String,
    pub partitions: BTreeMap<String, Region>,
    pub identity_sha256: BTreeMap<String, String>,
    pub retained_sha256: BTreeMap<String, String>,
}
pub struct BoundEnrollment {
    saved: SavedEnrollment,
}
impl SavedEnrollment {
    pub fn provenance(&self) -> &'static str {
        self.provenance
    }
    pub fn record(&self) -> &Record {
        &self.record
    }
    pub fn directory(&self) -> &Path {
        &self.root
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn rebind(
        self,
        observed: &ObservedHardware,
        session: &mut SessionGuard,
    ) -> Result<BoundEnrollment> {
        ensure!(
            matches!(session.phase(), Phase::Created | Phase::InputsVerified),
            "rebind must precede install phase transitions"
        );
        ensure!(
            observed.hwcode == 0x6580
                && observed.cid_encoding == "mt6580-legacy-le32-registers"
                && observed.cid == self.record.cid
                && observed.capacity == self.record.capacity
                && observed.partitions == self.record.partitions
                && observed.identity_sha256 == self.record.identity_sha256
                && self
                    .retained_sha256
                    .iter()
                    .all(|(name, hash)| observed.retained_sha256.get(name) == Some(hash)),
            "live hardware differs from retained enrollment"
        );
        session.checkpoint(
            &json!({"event":"imported_enrollment_rebound","enrollment_sha256":self.sha256}),
        )?;
        Ok(BoundEnrollment { saved: self })
    }
}
impl BoundEnrollment {
    pub fn enrollment(&self) -> &SavedEnrollment {
        &self.saved
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut file = crate::regular(path)?;
    ensure!(
        file.metadata()?.len() <= limit,
        "enrollment metadata exceeds bound"
    );
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "enrollment metadata grew");
    Ok(bytes)
}
fn validate(record: &Record) -> Result<()> {
    validate_record(record, false)
}
fn validate_record(record: &Record, legacy: bool) -> Result<()> {
    ensure!(
        record.schema == 1
            && record.kind
                == if legacy {
                    "couch-retained-python-enrollment"
                } else {
                    "couch-device-enrollment"
                }
            && record.model == "sanytron-ha100"
            && record.original_os == "Android",
        "not an original Android HA100 enrollment"
    );
    ensure!(
        valid_hex(&record.cid, 32) && record.cid != "0".repeat(32) && record.cid != "f".repeat(32),
        "invalid enrolled CID"
    );
    ensure!(
        record.capacity > 0
            && record.capacity <= 64 * 1024 * 1024 * 1024
            && record.capacity.is_multiple_of(512),
        "invalid enrolled capacity"
    );
    let wanted: BTreeSet<_> = STARTS
        .iter()
        .map(|(n, _)| (*n).to_string())
        .chain(["flashinfo".into()])
        .collect();
    ensure!(
        record.partitions.keys().cloned().collect::<BTreeSet<_>>() == wanted,
        "unexpected enrolled partition inventory"
    );
    for (index, (name, offset)) in STARTS.iter().enumerate() {
        let region = &record.partitions[*name];
        ensure!(
            region.offset == *offset,
            "partition differs from official HA100 profile"
        );
        let next = STARTS
            .get(index + 1)
            .map(|(_, n)| *n)
            .unwrap_or(record.partitions["flashinfo"].offset);
        ensure!(
            next > region.offset && region.size == next - region.offset,
            "partition size differs from official HA100 profile"
        );
    }
    for region in record.partitions.values() {
        ensure!(
            region.size > 0
                && region.offset.is_multiple_of(512)
                && region.size.is_multiple_of(512)
                && region
                    .offset
                    .checked_add(region.size)
                    .is_some_and(|n| n <= record.capacity),
            "invalid enrolled partition bounds"
        );
    }
    if legacy {
        let names = record
            .originals
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        ensure!(
            ["boot", "recovery", "logo"]
                .into_iter()
                .chain(IDENTITY)
                .all(|name| names.contains(name))
                && names
                    .iter()
                    .all(|name| ORIGINALS.contains(name) || *name == "userdata"),
            "incomplete legacy originals"
        );
    } else {
        ensure!(
            record
                .originals
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                == ORIGINALS.into_iter().collect(),
            "incomplete enrolled originals"
        );
    }
    ensure!(
        record
            .identity_sha256
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            == IDENTITY.into_iter().collect(),
        "incomplete calibration inventory"
    );
    for (name, file) in &record.originals {
        ensure!(
            file.file == format!("bootstrap-{name}.img")
                && file.size == record.partitions[name].size
                && file.size
                    <= if legacy && name == "userdata" {
                        16 * 1024 * 1024 * 1024
                    } else {
                        32 * 1024 * 1024
                    }
                && valid_hex(&file.sha256, 64),
            "invalid original backup inventory"
        );
        if IDENTITY.contains(&name.as_str()) {
            ensure!(
                record.identity_sha256[name] == file.sha256,
                "calibration receipt differs"
            );
        }
    }
    if legacy && record.android_identity.is_none() {
        return Ok(());
    }
    let identity = record
        .android_identity
        .as_ref()
        .context("missing original vendor identity")?;
    ensure!(
        !identity.device_id.is_empty()
            && identity.device_id.len() <= 256
            && !identity.device_id.chars().any(char::is_control),
        "missing original vendor identity"
    );
    for mac in [&identity.wifi_mac, &identity.bt_mac] {
        let clean = mac.replace(':', "").to_ascii_lowercase();
        ensure!(
            mac.len() == 17
                && mac.split(':').count() == 6
                && mac.split(':').all(|p| p.len() == 2)
                && valid_hex(&clean, 12)
                && clean != "0".repeat(12)
                && clean != "f".repeat(12),
            "invalid retained hardware address"
        );
    }
    Ok(())
}
fn journal(source: &Path, record: &Record, checksum: &str) -> Result<Vec<Vec<u8>>> {
    let mut receipts = BTreeSet::new();
    let mut events = Vec::new();
    let mut prior = Phase::Created;
    for sequence in 0..128 {
        let bytes = read(&source.join(format!("event-{sequence:05}.json")), 64 * 1024)?;
        let event: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            event["schema"] == 1 && event["sequence"] == sequence,
            "invalid enrollment journal sequence"
        );
        let phase: Phase = serde_json::from_value(event["phase"].clone())?;
        if sequence == 0 {
            ensure!(
                phase == Phase::Created && event["kind"] == "created",
                "missing session creation"
            );
        } else if event["kind"] == "transition" {
            ensure!(
                matches!(
                    (prior, phase),
                    (Phase::Created, Phase::InputsVerified)
                        | (Phase::InputsVerified, Phase::AndroidBound)
                        | (Phase::AndroidBound, Phase::OriginalsSaved)
                ),
                "invalid enrollment phase transition"
            );
        } else {
            ensure!(
                event["kind"] == "checkpoint" && phase == prior,
                "invalid enrollment checkpoint"
            );
        }
        let evidence = &event["evidence"];
        if evidence["event"] == "bootstrap_original_verified" {
            let name = evidence["target"]
                .as_str()
                .context("missing original target")?;
            let expected = record
                .originals
                .get(name)
                .context("unexpected original target")?;
            ensure!(
                phase == Phase::AndroidBound
                    && evidence["sha256"] == expected.sha256
                    && evidence["size"] == expected.size
                    && evidence["file"] == expected.file
                    && receipts.insert(name.to_string()),
                "invalid original verification checkpoint"
            );
        }
        events.push(bytes);
        if phase == Phase::OriginalsSaved {
            ensure!(
                evidence["event"] == "enrollment_complete"
                    && evidence["enrollment_sha256"] == checksum
                    && receipts.len() == 9,
                "incomplete or changed enrollment journal"
            );
            return Ok(events);
        }
        prior = phase;
    }
    anyhow::bail!("enrollment journal exceeds bound or never completed")
}
/// Import only copies verified originals; source is untouched, no old session resumes.
pub fn import(source: &Path, destination: &mut SessionGuard) -> Result<SavedEnrollment> {
    import_checked(
        source,
        destination,
        &[
            (
                "boot",
                8030464,
                "dda78c8ebe7cb82095b08a10c2a1f779cbdbebc53464aee34c85bb3a7382cad7",
            ),
            (
                "odmdtbo",
                37120,
                "a5cf1159f6e8c0a95bd1d3b8c1edaca3b2c9704642df50912dfe52df277f0575",
            ),
        ],
    )
}
fn import_checked(
    source: &Path,
    destination: &mut SessionGuard,
    prefixes: &[(&str, usize, &str)],
) -> Result<SavedEnrollment> {
    ensure!(
        matches!(destination.phase(), Phase::Created | Phase::InputsVerified),
        "import must precede device binding"
    );
    ensure!(
        fs::symlink_metadata(source)?.is_dir(),
        "enrollment source must be a real directory"
    );
    let bytes = read(&source.join("enrollment.json"), 65536)?;
    let record: Record = serde_json::from_slice(&bytes)?;
    validate(&record)?;
    let checksum = digest(&bytes);
    let events = journal(source, &record, &checksum)?;
    // Admit every opened snapshot before publishing any imported data.
    let mut files = Vec::new();
    let mut stock_prefixes = BTreeMap::new();
    for (name, entry) in &record.originals {
        let data = read(&source.join(&entry.file), entry.size)?;
        ensure!(
            data.len() as u64 == entry.size && digest(&data) == entry.sha256,
            "original backup changed"
        );
        for &(target, size, expected) in prefixes {
            if name == target {
                stock_prefixes.insert(
                    target,
                    data.len() >= size && digest(&data[..size]) == expected,
                );
            }
        }
        files.push((entry.file.clone(), data));
    }
    ensure!(
        native_stock_evidence(&record, &stock_prefixes),
        "saved originals do not establish a reviewed HA100 Android boot/overlay pair"
    );
    let root = destination.path().join("imported-enrollment");
    crate::private_dir(&root)?;
    for (name, data) in files
        .into_iter()
        .chain(std::iter::once(("enrollment.json".into(), bytes)))
        .chain(
            events
                .into_iter()
                .enumerate()
                .map(|(n, b)| (format!("event-{n:05}.json"), b)),
        )
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(root.join(name))?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    crate::session::sync_private_directory(&root)?;
    destination.checkpoint(&json!({"event":"android_enrollment_imported","enrollment_sha256":checksum,"live_hardware_rebound":false}))?;
    let retained_sha256 =
        BTreeMap::from([("odmdtbo".into(), record.originals["odmdtbo"].sha256.clone())]);
    Ok(SavedEnrollment {
        record,
        root,
        sha256: checksum,
        retained_sha256,
        provenance: "native-android-enrollment",
    })
}

// Called only after the complete native journal and each original file hash are
// verified. Reuse fresh enrollment's joint profile policy; never mix variants.
fn native_stock_evidence(record: &Record, prefixes: &BTreeMap<&str, bool>) -> bool {
    crate::enrollment::admitted_stock_pair(
        [
            prefixes.get("boot") == Some(&true),
            prefixes.get("odmdtbo") == Some(&true),
        ],
        [
            &record.originals["boot"].sha256,
            &record.originals["odmdtbo"].sha256,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn private_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        root
    }
    fn record() -> Record {
        let capacity = 8 * 1024 * 1024 * 1024;
        let mut partitions = BTreeMap::new();
        for (index, (name, offset)) in STARTS.iter().enumerate() {
            let end = STARTS
                .get(index + 1)
                .map(|(_, n)| *n)
                .unwrap_or(capacity - 1024 * 1024);
            partitions.insert(
                (*name).into(),
                Region {
                    offset: *offset,
                    size: end - offset,
                },
            );
        }
        partitions.insert(
            "flashinfo".into(),
            Region {
                offset: capacity - 1024 * 1024,
                size: 1024 * 1024,
            },
        );
        let originals = ORIGINALS
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Original {
                        file: format!("bootstrap-{name}.img"),
                        size: partitions[name].size,
                        sha256: "a".repeat(64),
                    },
                )
            })
            .collect();
        Record {
            schema: 1,
            kind: "couch-device-enrollment".into(),
            model: "sanytron-ha100".into(),
            cid: "12".repeat(16),
            capacity,
            partitions,
            identity_sha256: IDENTITY
                .into_iter()
                .map(|n| (n.into(), "a".repeat(64)))
                .collect(),
            android_identity: Some(AndroidIdentity {
                device_id: "fixture-vendor-id".into(),
                wifi_mac: "02:01:02:03:04:05".into(),
                bt_mac: "02:01:02:03:04:06".into(),
            }),
            original_os: "Android".into(),
            originals,
        }
    }
    fn observed(record: &Record) -> ObservedHardware {
        ObservedHardware {
            cid: record.cid.clone(),
            capacity: record.capacity,
            hwcode: 0x6580,
            cid_encoding: "mt6580-legacy-le32-registers".into(),
            partitions: record.partitions.clone(),
            identity_sha256: record.identity_sha256.clone(),
            retained_sha256: record
                .originals
                .get("odmdtbo")
                .map(|entry| BTreeMap::from([("odmdtbo".into(), entry.sha256.clone())]))
                .unwrap_or_default(),
        }
    }
    #[test]
    fn native_import_reuses_joint_stock_policy_without_relaxing_journal_or_hash_checks() {
        let mut saved = record();
        let latest = BTreeMap::from([("boot", true), ("odmdtbo", true)]);
        assert!(native_stock_evidence(&saved, &latest));
        saved.originals.get_mut("boot").unwrap().sha256 =
            "68f6baf03d3df9cf42503b6c7e205cb630ab0cb0bf64d2d93e19e0f551ef0e15".into();
        saved.originals.get_mut("odmdtbo").unwrap().sha256 =
            "13933a032fff5df271af7ce521a320c6a0653aa05a4ff652d35742eece37d760".into();
        assert!(native_stock_evidence(&saved, &BTreeMap::new()));
        saved.originals.get_mut("odmdtbo").unwrap().sha256 = "a".repeat(64);
        assert!(!native_stock_evidence(
            &saved,
            &BTreeMap::from([("boot", false), ("odmdtbo", true)])
        ));
        saved.originals.get_mut("boot").unwrap().sha256 = "b".repeat(64);
        assert!(!native_stock_evidence(&saved, &BTreeMap::new()));
    }
    #[test]
    fn only_exact_android_profile_and_original_inventory_are_accepted() {
        let good = record();
        validate(&good).unwrap();
        for change in 0..5 {
            let mut bad = good.clone();
            match change {
                0 => bad.original_os = "Couch".into(),
                1 => bad.partitions.get_mut("boot").unwrap().size -= 4096,
                2 => bad.originals.get_mut("logo").unwrap().file = "../logo.img".into(),
                3 => {
                    bad.identity_sha256.remove("nvram");
                }
                _ => bad.model = "mt6580".into(),
            };
            assert!(validate(&bad).is_err());
        }
    }
    #[test]
    fn imported_metadata_cannot_bind_another_device_or_changed_calibration() {
        for changed in 0..6 {
            let root = private_root();
            let mut session = SessionGuard::create(&root.path().join("new")).unwrap();
            let record = record();
            let mut live = observed(&record);
            match changed {
                1 => live.cid = "34".repeat(16),
                2 => {
                    live.identity_sha256.insert("nvram".into(), "b".repeat(64));
                }
                3 => live.partitions.get_mut("userdata").unwrap().size -= 4096,
                4 => {
                    live.retained_sha256.clear();
                }
                5 => {
                    live.retained_sha256
                        .insert("odmdtbo".into(), "b".repeat(64));
                }
                _ => {}
            };
            let saved = SavedEnrollment {
                retained_sha256: BTreeMap::from([(
                    "odmdtbo".into(),
                    record.originals["odmdtbo"].sha256.clone(),
                )]),
                record,
                root: root.path().join("saved"),
                sha256: "a".repeat(64),
                provenance: "fixture",
            };
            assert_eq!(saved.rebind(&live, &mut session).is_ok(), changed == 0);
            assert_eq!(session.phase(), Phase::Created);
        }
    }
    #[test]
    fn complete_journal_and_full_hashes_are_required_before_copy() {
        let root = private_root();
        let source = root.path().join("source");
        let mut original_session = SessionGuard::create(&source).unwrap();
        original_session
            .transition(Phase::InputsVerified, &json!({}))
            .unwrap();
        original_session
            .transition(Phase::AndroidBound, &json!({}))
            .unwrap();
        let mut record = record();
        for (name, entry) in &mut record.originals {
            let data = vec![7; entry.size as usize];
            entry.sha256 = digest(&data);
            fs::write(source.join(&entry.file), data).unwrap();
            if IDENTITY.contains(&name.as_str()) {
                record
                    .identity_sha256
                    .insert(name.clone(), entry.sha256.clone());
            }
            original_session.checkpoint(&json!({"event":"bootstrap_original_verified","target":name,"size":entry.size,"sha256":entry.sha256,"file":entry.file})).unwrap();
        }
        let metadata = serde_json::to_vec(&record).unwrap();
        let hash = digest(&metadata);
        fs::write(source.join("enrollment.json"), &metadata).unwrap();
        let mut incomplete = SessionGuard::create(&root.path().join("incomplete")).unwrap();
        assert!(import_checked(&source, &mut incomplete, &[]).is_err());
        assert!(!incomplete.path().join("imported-enrollment").exists());
        original_session
            .transition(
                Phase::OriginalsSaved,
                &json!({"event":"enrollment_complete","enrollment_sha256":hash}),
            )
            .unwrap();
        drop(original_session);
        let mut target = SessionGuard::create(&root.path().join("target")).unwrap();
        let prefix = digest(&[7; 8]);
        let imported = import_checked(
            &source,
            &mut target,
            &[("boot", 8, &prefix), ("odmdtbo", 8, &prefix)],
        )
        .unwrap();
        assert_eq!(imported.record().original_os, "Android");
        assert_eq!(target.phase(), Phase::Created);
        assert_eq!(fs::read(source.join("enrollment.json")).unwrap(), metadata);
        assert!(imported.rebind(&observed(&record), &mut target).is_ok());
        fs::write(source.join("bootstrap-logo.img"), b"corrupt").unwrap();
        let mut failed = SessionGuard::create(&root.path().join("failed")).unwrap();
        assert!(import_checked(
            &source,
            &mut failed,
            &[("boot", 8, &prefix), ("odmdtbo", 8, &prefix)]
        )
        .is_err());
        assert!(!failed.path().join("imported-enrollment").exists());
    }
    #[test]
    fn legacy_import_preserves_evidence_and_requires_independent_pin_and_live_overlay() {
        let root = private_root();
        let source = root.path().join("python-run");
        for dir in ["bootstrap", "private-image", "originals"] {
            fs::create_dir_all(source.join(dir)).unwrap();
        }
        let mut r = record();
        r.originals.remove("odmdtbo");
        let mut hashes = BTreeMap::new();
        for (name, entry) in &mut r.originals {
            let bytes = vec![7; entry.size as usize];
            entry.sha256 = digest(&bytes);
            fs::write(source.join("originals").join(format!("{name}.img")), bytes).unwrap();
            hashes.insert(name.clone(), entry.sha256.clone());
            if IDENTITY.contains(&name.as_str()) {
                r.identity_sha256.insert(name.clone(), entry.sha256.clone());
            }
        }
        let overlay = "d".repeat(64);
        let profile = json!({"schema":1,"model":"sanytron-ha100","purpose":"stock-baseline","private_candidate":true,"installable":false,"partitions":r.partitions,"images":{"boot":{"sha256":hashes["boot"]},"recovery":{"sha256":hashes["recovery"]},"odmdtbo":{"sha256":overlay}}});
        let profile_bytes = serde_json::to_vec(&profile).unwrap();
        let profile_hash = digest(&profile_bytes);
        let profile_path = root.path().join("trusted-profile.json");
        fs::write(&profile_path, profile_bytes).unwrap();
        let baseline = json!({"cid":r.cid,"capacity":r.capacity,"partitions":r.partitions,"identity_sha256":r.identity_sha256});
        let plan = json!({"schema":1,"cid":r.cid,"capacity":r.capacity,"partitions":r.partitions,"identity_sha256":r.identity_sha256,"nonce":"a".repeat(64),"manifest_sha256":"b".repeat(64),"stage_sha256":"c".repeat(64),"original_boot_sha256":hashes["boot"],"images":{"boot":{},"recovery":{},"logo":{},"userdata":{}},"skip_userdata_backup":true});
        let cid_bytes = (0..16)
            .map(|i| u8::from_str_radix(&r.cid[i * 2..i * 2 + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let mut stock_hashes = hashes.clone();
        stock_hashes.insert("odmdtbo".into(), overlay.clone());
        let bootstrap = json!({"schema":1,"purpose":"private-stock-to-ram-bootstrap","complete":true,"backup_complete":true,"usb_cleanup_verified":true,"identity_verified_after":true,"retained_verified_after":true,"write":"verified","stock_manifest_sha256":profile_hash,"stage_sha256":plan["stage_sha256"],"device":{"partitions":r.partitions,"capacity":r.capacity,"hwcode":0x6580,"cid_encoding":"mt6580-legacy-le32-registers","runtime_cid_sha256":digest(&cid_bytes)},"identity_sha256":r.identity_sha256,"originals":hashes,"stock_hashes":stock_hashes});
        let bootstrap_bytes = serde_json::to_vec(&bootstrap).unwrap();
        let receipt = json!({"schema":1,"complete":true,"journal_sha256":digest(&bootstrap_bytes),"identity_sha256":r.identity_sha256,"cid_sha256":digest(&cid_bytes),"originals":hashes});
        let receipt_bytes = serde_json::to_vec(&receipt).unwrap();
        let journal = json!({"schema":1,"backup_complete":true,"bootstrap_receipt_sha256":digest(&receipt_bytes),"plan_sha256":digest(&serde_json::to_vec(&plan).unwrap()),"nonce":plan["nonce"],"backups":hashes});
        for (name, bytes) in [
            (
                "bootstrap/baseline.json",
                serde_json::to_vec(&baseline).unwrap(),
            ),
            ("bootstrap/journal.json", bootstrap_bytes.clone()),
            ("bootstrap/backup-receipt.json", receipt_bytes),
            (
                "private-image/plan.json",
                serde_json::to_vec(&plan).unwrap(),
            ),
            (
                "originals/journal.json",
                serde_json::to_vec(&journal).unwrap(),
            ),
        ] {
            fs::write(source.join(name), bytes).unwrap();
        }
        let mut wrong_pin = SessionGuard::create(&root.path().join("wrong-pin")).unwrap();
        assert!(import_legacy(
            &source,
            &profile_path,
            &"0".repeat(64),
            &mut wrong_pin,
            |_, _, _| Ok(())
        )
        .is_err());
        assert!(!wrong_pin.path().join("imported-enrollment").exists());
        let mut target = SessionGuard::create(&root.path().join("target")).unwrap();
        let imported = import_legacy(
            &source,
            &profile_path,
            &profile_hash,
            &mut target,
            |_, _, _| Ok(()),
        )
        .unwrap();
        assert!(imported.record().android_identity.is_none());
        assert!(!imported.record().originals.contains_key("userdata"));
        assert!(!imported.record().originals.contains_key("odmdtbo"));
        assert!(!imported.directory().join("enrollment.json").exists());
        assert!(fs::read_dir(imported.directory()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("event-")));
        assert_eq!(
            fs::read(imported.directory().join("bootstrap-journal.json")).unwrap(),
            bootstrap_bytes
        );
        for hash in [None, Some("e".repeat(64)), Some(overlay.clone())] {
            let saved = SavedEnrollment {
                record: imported.record.clone(),
                root: imported.root.clone(),
                sha256: imported.sha256.clone(),
                retained_sha256: imported.retained_sha256.clone(),
                provenance: imported.provenance,
            };
            let mut live = observed(&r);
            if let Some(value) = &hash {
                live.retained_sha256.insert("odmdtbo".into(), value.clone());
            }
            assert_eq!(
                saved.rebind(&live, &mut target).is_ok(),
                hash == Some(overlay.clone())
            );
        }
        fs::write(source.join("originals/logo.img"), b"changed").unwrap();
        let mut corrupt = SessionGuard::create(&root.path().join("corrupt")).unwrap();
        assert!(import_legacy(
            &source,
            &profile_path,
            &profile_hash,
            &mut corrupt,
            |_, _, _| Ok(())
        )
        .is_err());
        assert!(!corrupt.path().join("imported-enrollment").exists());
    }
}
