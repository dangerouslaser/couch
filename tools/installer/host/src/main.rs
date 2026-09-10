use anyhow::{bail, Result};
use std::path::PathBuf;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|value| value == "--ui-smoke") {
        return ui_smoke(&args[1..]);
    }
    if args.len() != 3 || args[0] != "prepare-official" {
        bail!("usage: couch-installer-host prepare-official OFFICIAL_ZIP NEW_PRIVATE_DIRECTORY");
    }
    couch_installer_host::prepare(&PathBuf::from(&args[1]), &PathBuf::from(&args[2]))?;
    println!("Official owner inputs verified with native Rust. No USB or installation performed.");
    Ok(())
}

/// Explicitly device-free transport fixture. It cannot enter installation code.
fn ui_smoke(args: &[std::ffi::OsString]) -> Result<()> {
    use couch_installer_host::frontend::{Choice, Ui};
    #[cfg(unix)]
    let mut ui = {
        anyhow::ensure!(
            args == ["--events-fd", "3"],
            "expected private event socket"
        );
        Ui::inherited_socket(3)?
    };
    #[cfg(windows)]
    let mut ui = {
        anyhow::ensure!(args == ["--events-stdio"], "expected private event pipes");
        Ui::stdio()
    };
    ui.set_steps(vec!["Interface check".into()])?;
    ui.choose(
        "Device-free interface check",
        "This checks the terminal channel only.",
        &[Choice {
            label: "Continue".into(),
            detail: "No device access".into(),
        }],
    )?;
    let _secret = ui.input(
        "Test masked input",
        "Enter any fixture text, not a real password.",
        true,
    )?;
    ui.progress(
        0,
        "Native interface verified. No USB session started.",
        1,
        1,
    )?;
    ui.finish(0)
}
