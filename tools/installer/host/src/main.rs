use anyhow::{bail, Result};
use std::path::PathBuf;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 || args[0] != "prepare-official" {
        bail!("usage: couch-installer-host prepare-official OFFICIAL_ZIP NEW_PRIVATE_DIRECTORY");
    }
    couch_installer_host::prepare(&PathBuf::from(&args[1]), &PathBuf::from(&args[2]))?;
    println!("Official owner inputs verified with native Rust. No USB or installation performed.");
    Ok(())
}
