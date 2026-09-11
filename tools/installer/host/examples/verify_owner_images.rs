//! Offline release acceptance using actual admitted public and owner inputs.
//! Never opens USB or executes an installer worker. Outputs remain owner-private.
use anyhow::{ensure, Result};
use couch_installer_host::{assembly, public_inputs, session};
use std::{fs, io::Write, path::PathBuf};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 4,
        "usage: verify_owner_images CONFIG ARCHIVE PREPARED NEW_OUTPUT"
    );
    let paths: Vec<_> = args.iter().map(PathBuf::from).collect();
    let release = public_inputs::release(&paths[0])?;
    session::create_private_parent(&paths[3])?;
    let files = public_inputs::extract(&release, &paths[1], &paths[3].join("public"))?;
    let kernel = fs::read(&files["zImage"])?;
    let mut images = serde_json::Map::new();
    for (name, input) in [
        ("boot", "boot.cpio.gz"),
        ("recovery", "recovery.cpio.gz"),
        ("installer", "installer.cpio.gz"),
    ] {
        let ramdisk = assembly::owner_ramdisk(&fs::read(&files[input])?, &paths[2])?;
        let image = assembly::boot_image(
            &paths[2],
            if name == "recovery" {
                None
            } else {
                Some(&kernel)
            },
            &ramdisk,
        )?;
        let path = paths[3].join(format!("{name}.img"));
        let mut output = public_inputs::create(&path)?;
        output.write_all(&image)?;
        output.sync_all()?;
        ensure!(
            fs::read(&path)? == image,
            "assembled image readback differs"
        );
        images.insert(name.into(), serde_json::json!({"size":image.len(),"ramdisk_size":ramdisk.len(),"sha256":public_inputs::digest(&path)?}));
    }
    let receipt = serde_json::json!({"payload_source_commit":release.source_commit,"payload_sha256":release.payload.sha256,"device_access":false,"owner_local_outputs":true,"images":images});
    let mut output = public_inputs::create(&paths[3].join("assembly.json"))?;
    serde_json::to_writer_pretty(&mut output, &receipt)?;
    output.sync_all()?;
    println!("{receipt}");
    Ok(())
}
