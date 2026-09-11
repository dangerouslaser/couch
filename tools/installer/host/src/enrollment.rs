//! Fresh-device admission: Android CID, fixed official layout and independently
//! saved partition reads must all agree before the native host admits bootstrap.
use crate::{
    adapter::Worker,
    frontend::Ui,
    public_inputs::{create, decode, digest},
    session::SessionGuard,
    stage::CHUNK,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
pub const IDENTITY: [&str; 5] = ["nvdata", "nvram", "proinfo", "protect1", "protect2"];
pub const ORIGINALS: [&str; 4] = ["boot", "recovery", "odmdtbo", "logo"];
pub fn admit_layout(observed: &Value, cid: &str, prepared: &Path) -> Result<()> {
    ensure!(
        observed["hwcode"] == 0x6580
            && observed["cid_encoding"] == "mt6580-legacy-le32-registers"
            && decode(cid)?.len() == 16
            && observed["runtime_cid_sha256"] == format!("{:x}", Sha256::digest(decode(cid)?)),
        "Android and download-agent storage identity differ"
    );
    let pin: Value =
        serde_json::from_str(include_str!("../../../release/ha100_official_runtime.json"))?;
    let scatter = prepared.join("bootstrap/scatter.txt");
    ensure!(
        digest(&scatter)? == pin["members"]["scatter.txt"]["sha256"],
        "official layout changed"
    );
    let mut offsets = BTreeMap::new();
    for line in fs::read_to_string(scatter)?.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        ensure!(fields.len() == 2, "invalid official scatter");
        if !["preloader", "pgpt", "sgpt", "flashinfo"].contains(&fields[0]) {
            offsets.insert(
                fields[0].to_string(),
                u64::from_str_radix(fields[1].trim_start_matches("0x"), 16)?,
            );
        }
    }
    let partitions = observed["partitions"]
        .as_object()
        .context("missing observed layout")?;
    ensure!(
        partitions.len() == offsets.len() + 1
            && partitions.contains_key("flashinfo")
            && offsets.keys().all(|n| partitions.contains_key(n)),
        "unexpected stock partition inventory"
    );
    let capacity = observed["capacity"]
        .as_u64()
        .context("missing storage capacity")?;
    let mut ordered: Vec<_> = offsets.iter().collect();
    ordered.sort_by_key(|(_, offset)| *offset);
    ensure!(
        ordered
            .last()
            .is_some_and(|(name, _)| name.as_str() == "userdata"),
        "invalid fixed partition profile"
    );
    for (name, offset) in &ordered {
        ensure!(
            partitions[*name]["offset"] == **offset,
            "partition offset differs from official profile"
        );
    }
    for pair in ordered.windows(2) {
        let (name, start) = pair[0];
        let (_, end) = pair[1];
        ensure!(
            partitions[name]["size"] == end - start,
            "fixed partition length differs from official profile"
        );
    }
    let mut ranges = Vec::new();
    for region in partitions.values() {
        let offset = region["offset"].as_u64().context("invalid offset")?;
        let size = region["size"].as_u64().context("invalid size")?;
        ensure!(
            size > 0
                && offset.is_multiple_of(512)
                && size.is_multiple_of(512)
                && offset.checked_add(size).is_some_and(|end| end <= capacity),
            "partition exceeds capacity"
        );
        ranges.push((offset, offset + size));
    }
    ranges.sort();
    ensure!(
        ranges.windows(2).all(|v| v[0].1 <= v[1].0),
        "overlapping partition layout"
    );
    ensure!(
        partitions["userdata"]["offset"].as_u64().unwrap()
            + partitions["userdata"]["size"].as_u64().unwrap()
            == partitions["flashinfo"]["offset"],
        "userdata boundary differs"
    );
    Ok(())
}
pub fn event(worker: &mut Worker, ui: &mut Ui, phase: usize) -> Result<Value> {
    loop {
        let value = worker.event()?;
        if value["event"] != "progress" {
            return Ok(value);
        }
        let done = value["done"].as_u64().context("invalid USB progress")?;
        let total = value["total"].as_u64().context("invalid USB progress")?;
        ensure!(
            done <= total && total <= 16 * 1024 * 1024 * 1024,
            "invalid USB progress bounds"
        );
        let name = value["target"]
            .as_str()
            .context("invalid USB progress target")?;
        ensure!(
            IDENTITY.contains(&name) || ORIGINALS.contains(&name),
            "invalid USB progress target"
        );
        ui.progress(phase, &format!("Verifying {name}"), done, total)?;
    }
}

pub fn capture(
    worker: &mut Worker,
    device: &Value,
    session: &mut SessionGuard,
    ui: &mut Ui,
) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    for name in IDENTITY.into_iter().chain(ORIGINALS) {
        let size = device["partitions"][name]["size"]
            .as_u64()
            .context("missing original partition")?;
        ensure!(
            size > 0 && size <= 256 * 1024 * 1024,
            "unexpected bootstrap backup size"
        );
        let path = session.path().join(format!("bootstrap-{name}.img"));
        let mut output = create(&path)?;
        let expected = worker.operation(Duration::from_secs(1800), |w| {
            w.send(&json!({"op":"read","target":name}))?;
            ensure!(
                w.event()? == json!({"event":"partition","target":name,"size":size}),
                "original partition header differs"
            );
            let mut done = 0;
            let mut full = Sha256::new();
            while done < size {
                let count = (size - done).min(CHUNK as u64) as usize;
                ui.progress(2, &format!("Saving original {name}"), done, size)?;
                let data = w.chunk(count)?;
                output.write_all(&data)?;
                full.update(&data);
                done += count as u64;
            }
            output.sync_all()?;
            let hash = format!("{:x}", full.finalize());
            ensure!(
                w.event()? == json!({"event":"read_complete","target":name,"sha256":hash}),
                "original transfer digest differs"
            );
            Ok(hash)
        })?;
        ensure!(
            digest(&path)? == expected,
            "saved original readback differs"
        );
        worker.operation(Duration::from_secs(1800), |w| {
            ui.progress(
                2,
                &format!("Independently verifying original {name}"),
                0,
                size,
            )?;
            w.send(&json!({"op":"hash","target":name}))?;
            ensure!(
                event(w, ui, 2)? == json!({"event":"hash","target":name,"sha256":expected}),
                "independent original device read differs"
            );
            Ok(())
        })?;
        session.checkpoint(&json!({"event":"bootstrap_original_verified","target":name,"size":size,"sha256":expected,"file":path.file_name().unwrap().to_str().unwrap()}))?;
        hashes.insert(name.into(), expected);
    }
    Ok(hashes)
}
pub fn stock_prefixes(session: &SessionGuard, prepared: &Path) -> Result<()> {
    for name in ["boot", "odmdtbo"] {
        let official = fs::read(prepared.join(format!("bootstrap/{name}.img")))?;
        let mut original = fs::File::open(session.path().join(format!("bootstrap-{name}.img")))?;
        let mut prefix = vec![0; official.len()];
        original.read_exact(&mut prefix)?;
        ensure!(
            prefix == official,
            "original stock image differs from reviewed HA100 firmware"
        );
    }
    Ok(())
}
pub fn original_boot(session: &SessionGuard) -> PathBuf {
    session.path().join("bootstrap-boot.img")
}
