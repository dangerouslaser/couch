//! Fixed public payload admission and owner-local official OTA download.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Duration,
};
const MAX: u64 = 2 * 1024 * 1024 * 1024;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blob {
    pub size: u64,
    pub sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub format: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub schema: u32,
    pub kind: String,
    pub model: String,
    pub version: String,
    pub payload: Payload,
    pub source_commit: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    kind: String,
    version: String,
    source_commit: String,
    files: BTreeMap<String, Blob>,
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn decode(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len().is_multiple_of(2)
            && value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid hex bytes"
    );
    (0..value.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&value[i..i + 2], 16)?))
        .collect()
}
pub fn digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    hash(&mut file)
}
fn hash(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut result = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        result.update(&buffer[..n]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(format!("{:x}", result.finalize()))
}
pub fn create(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
fn blob_valid(blob: &Blob) -> Result<()> {
    ensure!(
        blob.size > 0
            && blob.size <= MAX
            && blob.sha256.len() == 64
            && decode(&blob.sha256)?.len() == 32,
        "invalid artifact pin"
    );
    Ok(())
}
pub fn release(path: &Path) -> Result<Release> {
    ensure!(
        fs::symlink_metadata(path)?.is_file() && fs::metadata(path)?.len() <= 65536,
        "invalid release descriptor"
    );
    let result: Release = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        result.schema == 1
            && result.kind == "couch-native-installer-release"
            && result.model == "sanytron-ha100"
            && result.payload.format == "tar.gz"
            && !result.version.is_empty()
            && result.version.len() <= 128
            && result.source_commit.len() == 40
            && decode(&result.source_commit)?.len() == 20,
        "unsupported release descriptor"
    );
    blob_valid(&Blob {
        size: result.payload.size,
        sha256: result.payload.sha256.clone(),
    })?;
    Ok(result)
}
fn download(
    url: &str,
    blob: &Blob,
    destination: &Path,
    official: bool,
    mut progress: impl FnMut(u64, u64) -> Result<()>,
) -> Result<()> {
    blob_valid(blob)?;
    let url = reqwest::Url::parse(url)?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && (url.scheme() == "https" || official && url.scheme() == "http"),
        "unsupported artifact URL"
    );
    let allow_http = official;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(1800))
        .redirect(reqwest::redirect::Policy::custom(move |a| {
            if a.previous().len() > 5
                || a.url().scheme() != "https" && !(allow_http && a.url().scheme() == "http")
            {
                a.stop()
            } else {
                a.follow()
            }
        }))
        .build()?;
    let mut response = client.get(url).send()?.error_for_status()?;
    ensure!(
        response.content_length().is_none_or(|n| n == blob.size),
        "artifact length differs"
    );
    let mut output = create(destination)?;
    let mut full = Sha256::new();
    let mut buffer = [0; 65536];
    let mut done = 0;
    progress(0, blob.size)?;
    loop {
        let n = response.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        done += n as u64;
        ensure!(done <= blob.size, "artifact exceeds pin");
        full.update(&buffer[..n]);
        output.write_all(&buffer[..n])?;
        progress(done, blob.size)?;
    }
    output.sync_all()?;
    ensure!(
        done == blob.size
            && format!("{:x}", full.finalize()) == blob.sha256
            && hash(&mut output)? == blob.sha256,
        "artifact hash differs"
    );
    Ok(())
}
pub fn official(destination: &Path, progress: impl FnMut(u64, u64) -> Result<()>) -> Result<()> {
    let pin: serde_json::Value =
        serde_json::from_str(include_str!("../../../release/ha100_official_runtime.json"))?;
    download(
        pin["url"].as_str().unwrap(),
        &Blob {
            size: pin["size"].as_u64().unwrap(),
            sha256: pin["sha256"].as_str().unwrap().into(),
        },
        destination,
        true,
        progress,
    )
}
pub fn payload(
    release: &Release,
    session: &Path,
    local: Option<&Path>,
    mut progress: impl FnMut(u64, u64) -> Result<()>,
) -> Result<BTreeMap<String, PathBuf>> {
    let archive = session.join("public-payload.tar.gz");
    let blob = Blob {
        size: release.payload.size,
        sha256: release.payload.sha256.clone(),
    };
    if let Some(local) = local {
        copy_local(local, &archive, &blob, &mut progress)?;
    } else {
        download(&release.payload.url, &blob, &archive, false, progress)?;
    }
    extract(release, &archive, &session.join("public-inputs"))
}
fn copy_local(
    source: &Path,
    destination: &Path,
    blob: &Blob,
    progress: &mut impl FnMut(u64, u64) -> Result<()>,
) -> Result<()> {
    blob_valid(blob)?;
    ensure!(
        std::fs::symlink_metadata(source)?.is_file(),
        "local OS package must be a regular file"
    );
    let mut input = File::open(source)?;
    ensure!(
        input.metadata()?.is_file() && input.metadata()?.len() == blob.size,
        "local OS package size differs"
    );
    let mut output = create(destination)?;
    let mut done = 0;
    let mut buffer = [0; 65536];
    progress(0, blob.size)?;
    while done < blob.size {
        let count = ((blob.size - done) as usize).min(buffer.len());
        input.read_exact(&mut buffer[..count])?;
        output.write_all(&buffer[..count])?;
        done += count as u64;
        progress(done, blob.size)?;
    }
    ensure!(
        input.read(&mut buffer[..1])? == 0 && hash(&mut output)? == blob.sha256,
        "local OS package digest differs"
    );
    output.sync_all()?;
    Ok(())
}
pub fn extract(
    release: &Release,
    archive: &Path,
    destination: &Path,
) -> Result<BTreeMap<String, PathBuf>> {
    let mut file = File::open(archive)?;
    ensure!(
        file.metadata()?.len() == release.payload.size
            && hash(&mut file)? == release.payload.sha256,
        "public archive changed"
    );
    fs::create_dir(destination)?;
    let required: BTreeSet<_> = [
        "userdata.ext4",
        "installer.cpio.gz",
        "boot.cpio.gz",
        "recovery.cpio.gz",
        "zImage",
        "logo.bgra",
    ]
    .into_iter()
    .collect();
    let mut found = BTreeMap::new();
    let mut total = 0;
    let mut manifest = None;
    {
        let decoded = flate2::bufread::GzDecoder::new(std::io::BufReader::new(&mut file));
        let mut tar = tar::Archive::new(decoded.take(MAX));
        for entry in tar.entries()?.raw(true) {
            let mut entry = entry?;
            ensure!(
                entry.header().entry_type().is_file(),
                "public payload contains a non-file member"
            );
            let path = entry.path()?.into_owned();
            let name = path
                .to_str()
                .context("invalid public member name")?
                .to_string();
            ensure!(
                required.contains(name.as_str()) || name == "logo.bgra" || name == "manifest.json",
                "public payload member not allowed"
            );
            ensure!(
                name == path.file_name().unwrap().to_str().unwrap() && !found.contains_key(&name),
                "duplicate or nested public member"
            );
            let size = entry.size();
            total += size;
            ensure!(
                size > 0 && size <= MAX && total <= MAX && found.len() < 7,
                "public payload exceeds bound"
            );
            let target = destination.join(&name);
            let mut output = create(&target)?;
            let copied = std::io::copy(&mut entry, &mut output)?;
            output.sync_all()?;
            ensure!(copied == size, "truncated public member");
            if name == "manifest.json" {
                ensure!(size <= 65536, "public manifest too large");
                manifest = Some(serde_json::from_slice::<Manifest>(&fs::read(&target)?)?);
            }
            found.insert(name, target);
        }
        let mut remaining = tar.into_inner();
        let mut padding = Vec::new();
        remaining.by_ref().take(65537).read_to_end(&mut padding)?;
        ensure!(
            padding.len() <= 65536 && padding.iter().all(|b| *b == 0),
            "non-padding data after public tar end"
        );
        let decoded = remaining.into_inner();
        let mut compressed = decoded.into_inner();
        ensure!(
            compressed.fill_buf()?.is_empty(),
            "trailing compressed payload data"
        );
    }
    ensure!(
        hash(&mut file)? == release.payload.sha256,
        "public archive changed during extraction"
    );
    let manifest = manifest.context("public manifest missing")?;
    ensure!(
        manifest.schema == 1
            && manifest.kind == "couch-public-os-inputs"
            && manifest.version == release.version
            && manifest.source_commit == release.source_commit,
        "public manifest identity differs"
    );
    found.remove("manifest.json");
    ensure!(
        required.iter().all(|n| found.contains_key(*n)) && found.keys().eq(manifest.files.keys()),
        "public file inventory differs"
    );
    for (name, path) in &found {
        let expected = &manifest.files[name];
        blob_valid(expected)?;
        ensure!(
            fs::metadata(path)?.len() == expected.size && digest(path)? == expected.sha256,
            "public member hash differs"
        );
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture(extra: Option<&str>, corrupt: bool) -> (tempfile::TempDir, Release, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("payload.tar.gz");
        let names = [
            "userdata.ext4",
            "installer.cpio.gz",
            "boot.cpio.gz",
            "recovery.cpio.gz",
            "zImage",
            "logo.bgra",
        ];
        let sha = format!("{:x}", Sha256::digest(b"fixture"));
        let files: BTreeMap<_, _> = names
            .into_iter()
            .map(|name| {
                (
                    name,
                    json!({"size":7,"sha256":if corrupt{"0".repeat(64)}else{sha.clone()}}),
                )
            })
            .collect();
        let manifest=serde_json::to_vec(&json!({"schema":1,"kind":"couch-public-os-inputs","version":"fixture","source_commit":"a".repeat(40),"files":files})).unwrap();
        let gzip = flate2::write::GzEncoder::new(
            File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(gzip);
        fn append(tar: &mut tar::Builder<flate2::write::GzEncoder<File>>, name: &str, data: &[u8]) {
            let mut header = tar::Header::new_ustar();
            header.set_size(data.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            tar.append_data(&mut header, name, data).unwrap();
        }
        append(&mut tar, "manifest.json", &manifest);
        for name in names {
            append(&mut tar, name, b"fixture");
        }
        if let Some(extra) = extra {
            append(&mut tar, extra, b"private");
        }
        tar.into_inner().unwrap().finish().unwrap();
        let release = Release {
            schema: 1,
            kind: "couch-native-installer-release".into(),
            model: "sanytron-ha100".into(),
            version: "fixture".into(),
            source_commit: "a".repeat(40),
            payload: Payload {
                url: "https://example.invalid/payload".into(),
                size: fs::metadata(&path).unwrap().len(),
                sha256: digest(&path).unwrap(),
                format: "tar.gz".into(),
            },
        };
        (root, release, path)
    }
    #[test]
    fn local_payload_is_pinned_bounded_and_cancellable() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::write(&source, b"public fixture").unwrap();
        let blob = Blob {
            size: 14,
            sha256: digest(&source).unwrap(),
        };
        let output = root.path().join("copy");
        copy_local(&source, &output, &blob, &mut |_, _| Ok(())).unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"public fixture");
        let wrong = Blob {
            size: 14,
            sha256: "0".repeat(64),
        };
        assert!(
            copy_local(&source, &root.path().join("wrong"), &wrong, &mut |_, _| Ok(
                ()
            ))
            .is_err()
        );
        assert!(copy_local(
            &source,
            &root.path().join("cancel"),
            &blob,
            &mut |_, _| anyhow::bail!("cancelled")
        )
        .is_err());
        assert!(copy_local(
            root.path(),
            &root.path().join("directory"),
            &blob,
            &mut |_, _| Ok(())
        )
        .is_err());
    }
    #[test]
    fn fixed_public_inventory_excludes_owner_vendor_and_duplicate_members() {
        let (root, release, path) = fixture(None, false);
        assert_eq!(
            extract(&release, &path, &root.path().join("good"))
                .unwrap()
                .len(),
            6
        );
        for name in ["vendor-runtime.bin", "userdata.ext4", "nested/boot.cpio.gz"] {
            let (root, release, path) = fixture(Some(name), false);
            assert!(extract(&release, &path, &root.path().join("bad")).is_err());
        }
    }
    #[test]
    fn public_member_hash_and_trailing_compressed_bytes_are_checked() {
        let (root, release, path) = fixture(None, true);
        assert!(extract(&release, &path, &root.path().join("bad")).is_err());
        let (root, mut release, path) = fixture(None, false);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"hidden owner data")
            .unwrap();
        release.payload.size = fs::metadata(&path).unwrap().len();
        release.payload.sha256 = digest(&path).unwrap();
        assert!(extract(&release, &path, &root.path().join("trailing")).is_err());
    }
}
