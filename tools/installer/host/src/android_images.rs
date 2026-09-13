//! Structural admission of a remote's original Android boot and overlay images.
//!
//! The installer writes its own boot, recovery, logo, overlay and userdata, so
//! the Android version on the remote does not affect Couch. The saved boot and
//! overlay matter for one reason: they are the originals that a stock restore
//! writes back and that a reinstall on another computer imports. This module
//! checks that they are what an HA100 running Android carries, independent of
//! the firmware version:
//!
//! - `boot` is an Android boot image whose ramdisk is a gzip cpio archive with
//!   Android's `init.rc` at its root. Every Couch boot and installer image
//!   reuses the vendor header but carries a busybox ramdisk without `init.rc`,
//!   so a previous Couch install is never admitted as an Android original.
//! - `odmdtbo` is a MediaTek dtbo container whose flattened device tree names
//!   `mediatek,` compatibles.
//!
//! HA100 identity itself is established elsewhere: the exact partition layout,
//! the eMMC CID, the MT6580 hardware code and the ADB model.
use anyhow::{ensure, Context, Result};
use flate2::read::GzDecoder;
use std::io::Read;

pub const PARTITION_SIZE: usize = 16 * 1024 * 1024;
const BOOT_MAGIC: &[u8; 8] = b"ANDROID!";
const MTK_DTBO_MAGIC: u32 = 0x8816_8858;
const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_OFFSET: usize = 0x400;
const RAMDISK_LIMIT: u64 = 64 * 1024 * 1024;

fn le32(bytes: &[u8], offset: usize) -> Result<usize> {
    let raw = bytes
        .get(offset..offset + 4)
        .context("truncated image header")?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()) as usize)
}
fn be32(bytes: &[u8], offset: usize) -> Result<u32> {
    let raw = bytes
        .get(offset..offset + 4)
        .context("truncated image header")?;
    Ok(u32::from_be_bytes(raw.try_into().unwrap()))
}
fn aligned(value: usize, page: usize) -> Result<usize> {
    value
        .checked_add(page - 1)
        .map(|n| n / page * page)
        .context("image offset overflow")
}

/// Names of the entries of a newc cpio archive. Lenient on purpose: it only
/// walks headers and never trusts metadata beyond the bounds it needs.
fn cpio_names(archive: &[u8]) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut offset = 0;
    while let Some(header) = archive.get(offset..offset + 110) {
        ensure!(
            &header[..6] == b"070701" || &header[..6] == b"070702",
            "ramdisk is not a newc cpio archive"
        );
        let field = |index: usize| -> Result<usize> {
            let text = std::str::from_utf8(&header[6 + index * 8..14 + index * 8])?;
            Ok(usize::from_str_radix(text, 16)?)
        };
        let (size, name_size) = (field(6)?, field(11)?);
        ensure!(name_size > 0 && name_size <= 4096, "invalid cpio name size");
        let name_end = offset + 110 + name_size;
        let name = archive
            .get(offset + 110..name_end)
            .context("truncated cpio name")?;
        let name = std::str::from_utf8(&name[..name.len() - 1])?.to_owned();
        let data_start = aligned(name_end, 4)?;
        offset = aligned(data_start.checked_add(size).context("cpio overflow")?, 4)?;
        ensure!(offset <= archive.len(), "truncated cpio file");
        if name == "TRAILER!!!" {
            return Ok(names);
        }
        names.push(name);
        ensure!(names.len() <= 65536, "cpio archive exceeds bound");
    }
    anyhow::bail!("cpio archive has no trailer")
}

/// An Android boot image (header version 0, as MT6580 firmware uses) whose
/// gzip cpio ramdisk carries Android's `init.rc` at the archive root.
pub fn android_boot(image: &[u8]) -> Result<()> {
    ensure!(
        image.len() == PARTITION_SIZE,
        "boot original is not a complete partition"
    );
    ensure!(
        &image[..8] == BOOT_MAGIC,
        "boot original is not an Android boot image"
    );
    let kernel_size = le32(image, 8)?;
    let ramdisk_size = le32(image, 16)?;
    let page = le32(image, 36)?;
    ensure!(
        [2048, 4096, 8192, 16384].contains(&page) && kernel_size > 0 && ramdisk_size > 0,
        "boot original has an unsupported header geometry"
    );
    let ramdisk_start = aligned(page + kernel_size, page)?;
    let ramdisk = image
        .get(
            ramdisk_start
                ..ramdisk_start
                    .checked_add(ramdisk_size)
                    .context("boot overflow")?,
        )
        .context("boot original ramdisk exceeds the partition")?;
    let mut archive = Vec::new();
    GzDecoder::new(ramdisk)
        .take(RAMDISK_LIMIT + 1)
        .read_to_end(&mut archive)
        .context("boot original ramdisk is not gzip")?;
    ensure!(
        archive.len() as u64 <= RAMDISK_LIMIT,
        "boot original ramdisk exceeds bound"
    );
    let names = cpio_names(&archive)?;
    ensure!(
        names.iter().any(|name| name == "init.rc"),
        "boot original ramdisk is not Android (no init.rc); a Couch image is not an Android original"
    );
    Ok(())
}

/// A MediaTek dtbo container with a flattened device tree at 0x400 that names
/// `mediatek,` compatibles.
pub fn mediatek_overlay(image: &[u8]) -> Result<()> {
    ensure!(
        image.len() == PARTITION_SIZE,
        "overlay original is not a complete partition"
    );
    ensure!(
        be32(image, 0)? == MTK_DTBO_MAGIC && &image[8..12] == b"dtbo",
        "overlay original is not a MediaTek dtbo container"
    );
    ensure!(
        be32(image, FDT_OFFSET)? == FDT_MAGIC,
        "overlay original carries no device tree"
    );
    let total = be32(image, FDT_OFFSET + 4)? as usize;
    let tree = image
        .get(FDT_OFFSET..FDT_OFFSET.checked_add(total).context("overlay overflow")?)
        .filter(|tree| tree.len() >= 40)
        .context("overlay original device tree exceeds the partition")?;
    ensure!(
        tree.windows(9).any(|window| window == b"mediatek,"),
        "overlay original device tree is not a MediaTek overlay"
    );
    Ok(())
}

/// Both originals together; the evidence string is recorded in the journal.
pub fn android_originals(boot: &[u8], overlay: &[u8]) -> Result<&'static str> {
    android_boot(boot)?;
    mediatek_overlay(overlay)?;
    Ok("android-boot-image+mediatek-overlay")
}

#[cfg(test)]
pub(crate) mod fixtures {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    fn cpio(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut push = |name: &str, data: &[u8]| {
            let header = format!(
                "070701{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
                out.len() + 1, 0o100644, 0, 0, 1, 0, data.len(), 0, 0, 0, 0, name.len() + 1, 0
            );
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            while out.len() % 4 != 0 {
                out.push(0);
            }
            out.extend_from_slice(data);
            while out.len() % 4 != 0 {
                out.push(0);
            }
        };
        for (name, data) in entries {
            push(name, data);
        }
        push("TRAILER!!!", b"");
        out
    }
    pub(crate) fn boot_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
        gzip.write_all(&cpio(entries)).unwrap();
        let ramdisk = gzip.finish().unwrap();
        let kernel = vec![0xe1; 4000];
        let mut image = vec![0u8; super::PARTITION_SIZE];
        image[..8].copy_from_slice(b"ANDROID!");
        image[8..12].copy_from_slice(&(kernel.len() as u32).to_le_bytes());
        image[16..20].copy_from_slice(&(ramdisk.len() as u32).to_le_bytes());
        image[36..40].copy_from_slice(&2048u32.to_le_bytes());
        image[2048..2048 + kernel.len()].copy_from_slice(&kernel);
        let ramdisk_start = 2048 + kernel.len().div_ceil(2048) * 2048;
        image[ramdisk_start..ramdisk_start + ramdisk.len()].copy_from_slice(&ramdisk);
        image
    }
    /// A vendor Android boot image: Android ramdisk with init.rc.
    pub(crate) fn android_boot() -> Vec<u8> {
        boot_with(&[
            ("init.rc", b"on init\n"),
            ("default.prop", b"ro.debuggable=1\n"),
        ])
    }
    /// A Couch-style image: same header, busybox ramdisk, no init.rc.
    pub(crate) fn couch_boot() -> Vec<u8> {
        boot_with(&[
            ("bin/busybox", b"\x7fELF"),
            ("init", b"#!/bin/busybox sh\n"),
        ])
    }
    pub(crate) fn overlay_with(compatible: &[u8]) -> Vec<u8> {
        let mut image = vec![0u8; super::PARTITION_SIZE];
        image[..4].copy_from_slice(&super::MTK_DTBO_MAGIC.to_be_bytes());
        image[8..12].copy_from_slice(b"dtbo");
        let mut tree = Vec::new();
        tree.extend_from_slice(&super::FDT_MAGIC.to_be_bytes());
        let total = 40 + 8 + compatible.len().div_ceil(4) * 4 + 8;
        tree.extend_from_slice(&(total as u32).to_be_bytes());
        tree.extend_from_slice(&[0u8; 32]);
        tree.extend_from_slice(compatible);
        tree.resize(total, 0);
        image[super::FDT_OFFSET..super::FDT_OFFSET + total].copy_from_slice(&tree);
        image
    }
    pub(crate) fn mediatek_overlay() -> Vec<u8> {
        overlay_with(b"mediatek,mt6580\0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_boot_with_init_rc_is_admitted_and_couch_ramdisk_is_not() {
        android_boot(&fixtures::android_boot()).unwrap();
        let couch = android_boot(&fixtures::couch_boot()).unwrap_err();
        assert!(couch.to_string().contains("no init.rc"), "{couch}");
        // init.rc must sit at the archive root, not inside a nested directory.
        assert!(android_boot(&fixtures::boot_with(&[("system/init.rc", b"x")])).is_err());
    }

    #[test]
    fn boot_geometry_and_container_faults_are_rejected() {
        let good = fixtures::android_boot();
        let mut short = good.clone();
        short.truncate(PARTITION_SIZE - 1);
        assert!(android_boot(&short).is_err());
        let mut magic = good.clone();
        magic[0] = b'X';
        assert!(android_boot(&magic).is_err());
        let mut page = good.clone();
        page[36..40].copy_from_slice(&1000u32.to_le_bytes());
        assert!(android_boot(&page).is_err());
        let mut oversized = good.clone();
        oversized[16..20].copy_from_slice(&(PARTITION_SIZE as u32).to_le_bytes());
        assert!(android_boot(&oversized).is_err());
        // The ramdisk starts one page after the 4000-byte kernel: 2048 + 4096.
        let mut plain = good.clone();
        plain[6144..6144 + 4].copy_from_slice(b"0707");
        assert!(android_boot(&plain).is_err());
        assert!(android_boot(&vec![0; PARTITION_SIZE]).is_err());
    }

    #[test]
    fn overlay_requires_mediatek_dtbo_container_with_device_tree() {
        mediatek_overlay(&fixtures::mediatek_overlay()).unwrap();
        assert!(mediatek_overlay(&fixtures::overlay_with(b"qcom,sdm845\0")).is_err());
        let mut no_tree = fixtures::mediatek_overlay();
        no_tree[FDT_OFFSET] = 0;
        assert!(mediatek_overlay(&no_tree).is_err());
        let mut huge = fixtures::mediatek_overlay();
        huge[FDT_OFFSET + 4..FDT_OFFSET + 8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(mediatek_overlay(&huge).is_err());
        let mut tag = fixtures::mediatek_overlay();
        tag[8..12].copy_from_slice(b"dtbX");
        assert!(mediatek_overlay(&tag).is_err());
        assert!(mediatek_overlay(&vec![0; PARTITION_SIZE]).is_err());
        assert!(mediatek_overlay(&fixtures::mediatek_overlay()[..PARTITION_SIZE - 1]).is_err());
    }

    #[test]
    fn pair_evidence_names_both_structures() {
        assert_eq!(
            android_originals(&fixtures::android_boot(), &fixtures::mediatek_overlay()).unwrap(),
            "android-boot-image+mediatek-overlay"
        );
        assert!(android_originals(&fixtures::couch_boot(), &fixtures::mediatek_overlay()).is_err());
        assert!(android_originals(&fixtures::android_boot(), &fixtures::couch_boot()).is_err());
    }
}
