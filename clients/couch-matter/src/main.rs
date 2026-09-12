//! JSON command line for the Matter controller, for scripting and for checking
//! a device from a shell before wiring it into a room.
use couch_matter::{pairing, Command, Controller};
use std::path::PathBuf;

const USAGE: &str = "Usage: couch-matter [--dir DIR] discover | commission CODE NAME | nodes | refresh NODE | rename NODE NAME | remove NODE | lights | light NODE/ENDPOINT | on|off|toggle NODE/ENDPOINT | brightness NODE/ENDPOINT 0..100\nDIR defaults to $COUCH_MATTER_DIR or ./matter";

fn main() {
    if let Err(error) = run() {
        eprintln!("couch-matter: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = std::env::var("COUCH_MATTER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| "matter".into());
    if args.first().map(String::as_str) == Some("--dir") {
        if args.len() < 2 {
            return Err(USAGE.into());
        }
        dir = PathBuf::from(args.remove(1));
        args.remove(0);
    }
    let action = args.first().map(String::as_str).ok_or(USAGE)?;
    let arg = |i: usize| args.get(i).map(String::as_str).ok_or(USAGE);
    let node = |i: usize| -> Result<u64, Box<dyn std::error::Error>> {
        Ok(arg(i)?.parse::<u64>().map_err(|_| USAGE)?)
    };
    let controller = Controller::open(&dir)?;
    let value = match (action, args.len()) {
        ("discover", 1) => serde_json::to_value(controller.discover()?)?,
        ("commission", 3) => {
            eprintln!(
                "couch-matter: searching for {} and commissioning, up to a minute…",
                pairing::display(&pairing::normalize(arg(1)?)?)
            );
            serde_json::to_value(controller.commission(arg(1)?, arg(2)?)?)?
        }
        ("nodes", 1) => serde_json::to_value(controller.nodes())?,
        ("refresh", 2) => serde_json::to_value(controller.refresh(node(1)?)?)?,
        ("rename", 3) => serde_json::to_value(controller.rename(node(1)?, arg(2)?)?)?,
        ("remove", 2) => {
            serde_json::json!({"removed": true, "fabric_released": controller.remove(node(1)?)?})
        }
        ("lights", 1) => serde_json::to_value(controller.lights())?,
        ("light", 2) => serde_json::to_value(controller.light(arg(1)?)?)?,
        ("on", 2) => serde_json::to_value(controller.command(arg(1)?, Command::On)?)?,
        ("off", 2) => serde_json::to_value(controller.command(arg(1)?, Command::Off)?)?,
        ("toggle", 2) => serde_json::to_value(controller.command(arg(1)?, Command::Toggle)?)?,
        ("brightness", 3) => {
            let percent: u8 = arg(2)?.parse().map_err(|_| USAGE)?;
            serde_json::to_value(controller.command(arg(1)?, Command::Brightness(percent))?)?
        }
        _ => return Err(USAGE.into()),
    };
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}
