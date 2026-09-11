//! Fixed public payload admission and owner-local official OTA download.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
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
    progress: impl FnMut(u64, u64) -> Result<()>,
) -> Result<BTreeMap<String, PathBuf>> {
    let archive = session.join("public-payload.tar.gz");
    let blob = Blob {
        size: release.payload.size,
        sha256: release.payload.sha256.clone(),
    };
    download(&release.payload.url, &blob, &archive, false, progress)?;
    extract(release, &archive, &session.join("public-inputs"))
}
fn extract(
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
        let decoded = flate2::read::GzDecoder::new(&mut file);
        let mut tar = tar::Archive::new(decoded.take(MAX));
        for entry in tar.entries()? {
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
