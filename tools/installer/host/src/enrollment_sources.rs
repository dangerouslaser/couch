//! Discovery and remembered selection for the saved Android enrollment folder.
//!
//! Listing performs cheap structural classification only. It deliberately does
//! NOT assert that a candidate is valid, and it never hashes a retained backup
//! merely to draw a menu. The authoritative admission checks stay in
//! `saved_enrollment::import` and `saved_enrollment::import_legacy` and run
//! unchanged on whatever the operator finally picks.
//!
//! A remembered entry records a path and nothing else. Because it carries no
//! digest, it cannot claim that a folder is unchanged since a previous import,
//! and it can never stand in for validation. Anything offered here is verified
//! again, in full, before it is used.
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// How a candidate was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Recorded after a previous fully validated import. Path only.
    Remembered,
    /// An installer-owned session folder holding `enrollment.json`.
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub origin: Origin,
}

impl Candidate {
    /// Menu wording. It describes what was observed structurally and states
    /// that verification still happens, so nothing here reads as a trust claim.
    pub fn detail(&self) -> &'static str {
        match self.origin {
            Origin::Remembered => "Used for a previous verified import. Checked again in full now.",
            Origin::Native => {
                "Installer session containing enrollment.json. Checked again in full now."
            }
        }
    }
}

const RECORD: &str = "enrollment-source.json";
const RECORD_LIMIT: u64 = 8192;
const SESSION_PREFIX: &str = "install-";
/// Upper bound on directory entries examined, so a state root that has grown
/// unexpectedly large cannot turn drawing a menu into an unbounded walk.
const MAX_SCAN: usize = 4096;
/// Upper bound on offered candidates. `Ui::choose` rejects a list longer than
/// 128 options outright, and that rejection is a hard error that would strand
/// the operator with no route to the manual prompt. Staying well under the
/// limit keeps the menu readable and keeps manual entry reachable no matter
/// how many sessions have accumulated.
const MENU_LIMIT: usize = 32;

/// Treat two spellings of one folder as the same entry where the filesystem
/// can confirm it. Falls back to a literal comparison when it cannot.
fn same(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => left == right,
    }
}

/// A folder holding a current-Couch snapshot records a previous Couch
/// reinstall, never an Android enrollment. Offering one would silently
/// substitute the wrong baseline, so it is excluded here. It would also fail
/// admission later; this keeps it out of the operator's way entirely.
fn couch_snapshot(dir: &Path) -> bool {
    // symlink_metadata, not exists: a dangling or unreadable entry of that name
    // still marks the folder as a Couch reinstall record, and `exists` would
    // quietly report false for both.
    fs::symlink_metadata(dir.join("current-couch-snapshot.json")).is_ok()
}

fn native_enrollment(dir: &Path) -> bool {
    dir.join("enrollment.json").is_file() && !couch_snapshot(dir)
}

/// Read the remembered path. Returns `None` for a missing, oversized,
/// unparsable, wrong-schema or empty record rather than guessing.
pub fn remembered(state_root: &Path) -> Option<PathBuf> {
    let path = state_root.join(RECORD);
    let meta = fs::symlink_metadata(&path).ok()?;
    // Require a regular file before reading. A symlink reports its own length
    // here while the read that follows would resolve the target, so accepting
    // one would let an arbitrarily large file past the size bound.
    if !meta.file_type().is_file() || meta.len() > RECORD_LIMIT {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
    if value.get("schema")?.as_u64()? != 1 {
        return None;
    }
    let recorded = value.get("path")?.as_str()?;
    if recorded.is_empty() {
        return None;
    }
    Some(PathBuf::from(recorded))
}

/// Candidates to offer, remembered entry first, then installer sessions in a
/// deterministic order. An empty result means the caller should fall back to
/// the manual prompt.
///
/// The list is capped at `MENU_LIMIT`. Manual entry stays reachable, so a cap
/// costs an operator a scroll rather than a route to the folder they wanted.
pub fn discover(state_root: &Path, remembered: Option<PathBuf>) -> Vec<Candidate> {
    let mut found = Vec::new();
    if let Some(path) = remembered {
        // The Couch-snapshot exclusion applies here too. A remembered entry is
        // not re-checked for enrollment.json, because a legacy Python export
        // legitimately lacks one, but a folder that has become a Couch
        // reinstall record is never an Android enrollment.
        if path.is_dir() && !couch_snapshot(&path) {
            found.push(Candidate {
                path,
                origin: Origin::Remembered,
            });
        }
    }
    let mut sessions = Vec::new();
    if let Ok(entries) = fs::read_dir(state_root) {
        for entry in entries.flatten().take(MAX_SCAN) {
            let path = entry.path();
            let named = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(SESSION_PREFIX));
            if named && native_enrollment(&path) {
                sessions.push(path);
            }
        }
    }
    // Sort before truncating, so the cap keeps the same entries every run
    // rather than whatever the filesystem happened to enumerate first.
    sessions.sort();
    for path in sessions {
        if found.len() >= MENU_LIMIT {
            break;
        }
        if !found.iter().any(|other| same(&other.path, &path)) {
            found.push(Candidate {
                path,
                origin: Origin::Native,
            });
        }
    }
    found
}

/// Record the folder just imported. Call only after a fully successful,
/// fully validated import.
///
/// Path only: no digest, no copied record fields, nothing that could later be
/// mistaken for evidence. Best effort by design, because failing to write a
/// convenience cache must never fail an installation that already succeeded.
pub fn remember(state_root: &Path, source: &Path) {
    let Some(text) = source.to_str() else {
        return;
    };
    let Ok(bytes) = serde_json::to_vec(&json!({"schema": 1, "path": text})) else {
        return;
    };
    let path = state_root.join(RECORD);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(&path) {
        use std::io::Write;
        let _ = file.write_all(&bytes);
        let _ = file.sync_all();
    }
}

/// Index of the trailing "enter a path myself" option. The manual option is
/// always present, so a lone candidate still needs a deliberate selection and
/// is never applied on the operator's behalf.
pub fn manual_index(candidates: &[Candidate]) -> usize {
    candidates.len()
}

/// Resolve a chosen menu index. `None` means the operator asked to type a path.
pub fn resolve(candidates: &[Candidate], picked: usize) -> Option<PathBuf> {
    candidates.get(picked).map(|found| found.path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn session(root: &Path, name: &str, files: &[&str]) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        for file in files {
            fs::write(dir.join(file), b"{}").unwrap();
        }
        dir
    }

    #[test]
    fn no_candidates_leaves_the_operator_at_the_manual_prompt() {
        let root = TempDir::new().unwrap();
        session(root.path(), "install-empty", &[]);
        session(root.path(), "unrelated-folder", &["enrollment.json"]);
        assert!(discover(root.path(), None).is_empty());
    }

    #[test]
    fn a_session_holding_enrollment_json_is_offered() {
        let root = TempDir::new().unwrap();
        let dir = session(root.path(), "install-aaaa", &["enrollment.json"]);
        let found = discover(root.path(), None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, dir);
        assert_eq!(found[0].origin, Origin::Native);
    }

    #[test]
    fn a_current_couch_snapshot_is_never_offered_as_an_android_enrollment() {
        let root = TempDir::new().unwrap();
        session(
            root.path(),
            "install-couch",
            &["current-couch-snapshot.json"],
        );
        assert!(discover(root.path(), None).is_empty());
    }

    #[test]
    fn a_session_holding_both_records_is_excluded_rather_than_guessed_at() {
        let root = TempDir::new().unwrap();
        session(
            root.path(),
            "install-both",
            &["enrollment.json", "current-couch-snapshot.json"],
        );
        assert!(discover(root.path(), None).is_empty());
    }

    #[test]
    fn several_sessions_are_all_offered_in_a_deterministic_order() {
        let root = TempDir::new().unwrap();
        let second = session(root.path(), "install-bbbb", &["enrollment.json"]);
        let first = session(root.path(), "install-aaaa", &["enrollment.json"]);
        let found = discover(root.path(), None);
        assert_eq!(
            found.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(discover(root.path(), None), found);
    }

    #[test]
    fn a_remembered_folder_outside_installer_state_is_offered_first() {
        let root = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let legacy = elsewhere.path().join("python-run");
        fs::create_dir_all(&legacy).unwrap();
        let native = session(root.path(), "install-aaaa", &["enrollment.json"]);
        let found = discover(root.path(), Some(legacy.clone()));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].origin, Origin::Remembered);
        assert_eq!(found[0].path, legacy);
        assert_eq!(found[1].path, native);
    }

    #[test]
    fn a_remembered_folder_that_is_gone_is_dropped_without_failing() {
        let root = TempDir::new().unwrap();
        let native = session(root.path(), "install-aaaa", &["enrollment.json"]);
        let found = discover(root.path(), Some(root.path().join("vanished")));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, native);
        assert_eq!(found[0].origin, Origin::Native);
    }

    #[test]
    fn a_remembered_folder_already_discovered_is_listed_once() {
        let root = TempDir::new().unwrap();
        let native = session(root.path(), "install-aaaa", &["enrollment.json"]);
        let found = discover(root.path(), Some(native.clone()));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].origin, Origin::Remembered);
        assert_eq!(found[0].path, native);
    }

    #[test]
    fn an_unusable_remembered_record_is_ignored_rather_than_guessed_at() {
        let root = TempDir::new().unwrap();
        assert_eq!(remembered(root.path()), None);
        for body in [
            "not json".to_string(),
            json!({"path": "/tmp/x"}).to_string(),
            json!({"schema": 2, "path": "/tmp/x"}).to_string(),
            json!({"schema": 1}).to_string(),
            json!({"schema": 1, "path": ""}).to_string(),
            json!({"schema": 1, "path": 7}).to_string(),
        ] {
            fs::write(root.path().join(RECORD), body.as_bytes()).unwrap();
            assert_eq!(remembered(root.path()), None);
        }
        fs::write(root.path().join(RECORD), vec![b'x'; 9000]).unwrap();
        assert_eq!(remembered(root.path()), None);
    }

    #[test]
    fn remembering_stores_a_path_only_and_no_trust_material() {
        let root = TempDir::new().unwrap();
        let chosen = session(root.path(), "install-aaaa", &["enrollment.json"]);
        remember(root.path(), &chosen);
        assert_eq!(remembered(root.path()), Some(chosen.clone()));
        let raw = fs::read_to_string(root.path().join(RECORD)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 2);
        assert!(object.contains_key("schema") && object.contains_key("path"));
        for forbidden in ["sha256", "cid", "originals", "digest", "identity_sha256"] {
            assert!(!raw.contains(forbidden), "{forbidden} must not be recorded");
        }
    }

    #[test]
    fn remembering_replaces_the_previous_entry_without_accumulating() {
        let root = TempDir::new().unwrap();
        let first = session(root.path(), "install-aaaa", &["enrollment.json"]);
        let second = session(root.path(), "install-bbbb", &["enrollment.json"]);
        remember(root.path(), &first);
        remember(root.path(), &second);
        assert_eq!(remembered(root.path()), Some(second));
    }

    #[test]
    fn a_single_candidate_still_requires_a_deliberate_selection() {
        let root = TempDir::new().unwrap();
        let native = session(root.path(), "install-aaaa", &["enrollment.json"]);
        let found = discover(root.path(), None);
        assert_eq!(manual_index(&found), 1);
        assert_eq!(resolve(&found, 0), Some(native));
        assert_eq!(resolve(&found, manual_index(&found)), None);
    }

    #[test]
    fn the_manual_option_stays_reachable_when_candidates_are_ambiguous() {
        let root = TempDir::new().unwrap();
        session(root.path(), "install-aaaa", &["enrollment.json"]);
        session(root.path(), "install-bbbb", &["enrollment.json"]);
        let found = discover(root.path(), None);
        assert_eq!(found.len(), 2);
        assert_eq!(manual_index(&found), 2);
        assert!(resolve(&found, 0).is_some());
        assert!(resolve(&found, 1).is_some());
        assert_eq!(resolve(&found, 2), None);
        assert_eq!(resolve(&found, 99), None);
    }

    #[test]
    fn the_offered_list_stays_within_the_frontend_choice_limit() {
        // Ui::choose refuses a list longer than 128 options, and that refusal
        // is a hard error with no fallback, so an operator who has accumulated
        // many sessions must not be locked out of the reinstall flow.
        let root = TempDir::new().unwrap();
        for n in 0..200 {
            session(
                root.path(),
                &format!("install-{n:04}"),
                &["enrollment.json"],
            );
        }
        let found = discover(root.path(), None);
        assert!(found.len() <= MENU_LIMIT);
        let offered = manual_index(&found) + 1;
        assert!(
            offered <= 128,
            "the menu must fit the frontend choice limit"
        );
        assert_eq!(resolve(&found, manual_index(&found)), None);
    }

    #[test]
    fn the_capped_list_keeps_the_same_entries_on_every_run() {
        let root = TempDir::new().unwrap();
        for n in 0..200 {
            session(
                root.path(),
                &format!("install-{n:04}"),
                &["enrollment.json"],
            );
        }
        let first = discover(root.path(), None);
        assert_eq!(discover(root.path(), None), first);
        let mut expected: Vec<_> = (0..200)
            .map(|n| root.path().join(format!("install-{n:04}")))
            .collect();
        expected.sort();
        expected.truncate(MENU_LIMIT);
        assert_eq!(
            first.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn a_remembered_folder_holding_a_couch_snapshot_is_not_offered() {
        let root = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let stale = elsewhere.path().join("was-an-export");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("current-couch-snapshot.json"), b"{}").unwrap();
        assert!(discover(root.path(), Some(stale)).is_empty());
    }

    #[test]
    fn a_remembered_legacy_export_without_enrollment_json_is_still_offered() {
        let root = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let legacy = elsewhere.path().join("python-run");
        fs::create_dir_all(&legacy).unwrap();
        let found = discover(root.path(), Some(legacy.clone()));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, legacy);
        assert_eq!(found[0].origin, Origin::Remembered);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_record_cannot_slip_past_the_size_bound() {
        let root = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let big = elsewhere.path().join("big.json");
        fs::write(&big, vec![b'x'; 9000]).unwrap();
        std::os::unix::fs::symlink(&big, root.path().join(RECORD)).unwrap();
        assert_eq!(remembered(root.path()), None);
        // Even a small, well-formed target is refused: the record must be a
        // regular file in installer state, not a pointer out of it.
        let small = elsewhere.path().join("small.json");
        fs::write(&small, json!({"schema": 1, "path": "/tmp/x"}).to_string()).unwrap();
        fs::remove_file(root.path().join(RECORD)).unwrap();
        std::os::unix::fs::symlink(&small, root.path().join(RECORD)).unwrap();
        assert_eq!(remembered(root.path()), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_couch_snapshot_entry_still_excludes_the_folder() {
        let root = TempDir::new().unwrap();
        let dir = session(root.path(), "install-aaaa", &["enrollment.json"]);
        std::os::unix::fs::symlink(
            dir.join("nothing-here"),
            dir.join("current-couch-snapshot.json"),
        )
        .unwrap();
        assert!(discover(root.path(), None).is_empty());
    }

    #[test]
    fn menu_wording_promises_verification_and_never_claims_validity() {
        let root = TempDir::new().unwrap();
        session(root.path(), "install-aaaa", &["enrollment.json"]);
        let elsewhere = TempDir::new().unwrap();
        for candidate in discover(root.path(), Some(elsewhere.path().to_path_buf())) {
            let detail = candidate.detail().to_lowercase();
            assert!(detail.contains("checked again in full"));
            assert!(!detail.contains("valid") && !detail.contains("trusted"));
        }
    }
}
