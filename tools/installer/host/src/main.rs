use anyhow::{bail, Result};
use std::path::PathBuf;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|value| value == "prepare-dependencies" || value == "--dependency-smoke")
    {
        anyhow::ensure!(
            args.len() == 2,
            "usage: couch-installer-host prepare-dependencies NEW_PRIVATE_PARENT"
        );
        use couch_installer_host::{dependencies, session};
        let parent = PathBuf::from(&args[1]);
        session::create_private_parent(&parent)?;
        let guard = session::SessionGuard::create(&parent.join("run"))?;
        let mut last = String::new();
        let prepared =
            dependencies::prepare(&guard, dependencies::host_platform()?, |label, _, _| {
                if label != last {
                    eprintln!("{label}…");
                    last = label.into();
                }
                Ok(())
            })?;
        if args[0] == "--dependency-smoke" {
            println!("{}", dependencies::smoke(&prepared)?);
        } else {
            println!(
                "{}",
                serde_json::json!({"runtime":prepared.runtime_root,"runtime_receipt_sha256":prepared.runtime_receipt_sha256,"adb":prepared.adb,"owner_da":prepared.owner_da,"usb_opened":false})
            );
        }
        return Ok(());
    }
    if args.first().is_some_and(|value| value == "--ui-smoke") {
        return ui_smoke(&args[1..]);
    }
    if args
        .iter()
        .any(|v| v == "--events-fd" || v == "--events-stdio")
    {
        return installer(&args);
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

fn installer(args: &[std::ffi::OsString]) -> Result<()> {
    use couch_installer_host::frontend::Ui;
    let mut config = None;
    let mut events = false;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--config" && config.is_none() && index + 1 < args.len() {
            config = Some(PathBuf::from(&args[index + 1]));
            index += 2;
        } else if cfg!(unix)
            && args[index] == "--events-fd"
            && !events
            && args.get(index + 1).is_some_and(|v| v == "3")
        {
            events = true;
            index += 2;
        } else if cfg!(windows) && args[index] == "--events-stdio" && !events {
            events = true;
            index += 1;
        } else {
            bail!("unsupported native installer arguments");
        }
    }
    anyhow::ensure!(events, "private installer event channel missing");
    #[cfg(unix)]
    let mut ui = Ui::inherited_socket(3)?;
    #[cfg(windows)]
    let mut ui = Ui::stdio();
    let result = couch_installer_host::orchestrator::run(&mut ui, config.as_deref());
    let _ = ui.finish(if result.is_ok() { 0 } else { 1 });
    result
}
