use couch_denon::{Client, Command, Settings};
fn main() {
    if let Err(e) = run() {
        eprintln!("couch-denon: {e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let host = args.next().ok_or(
        "Usage: couch-denon HOST [status|sources|watch|on|off|mute|unmute|up|down|volume DB|input ID]",
    )?;
    let mut client = Client::connect(&Settings { host, port: 23 })?;
    let action = args.next().unwrap_or("status".into());
    let state = match action.as_str() {
        "status" => client.status()?,
        "sources" => {
            println!("{}", serde_json::to_string(&client.sources()?)?);
            return Ok(());
        }
        "watch" => {
            println!("{}", serde_json::to_string(&client.status()?)?);
            loop {
                if let Some(s) = client.next_event(std::time::Duration::from_secs(30))? {
                    println!("{}", serde_json::to_string(&s)?);
                }
            }
        }
        action => client.command(match action {
            "on" => Command::Power(true),
            "off" => Command::Power(false),
            "mute" => Command::Mute(true),
            "unmute" => Command::Mute(false),
            "up" => Command::VolumeUp,
            "down" => Command::VolumeDown,
            "volume" => Command::VolumeDb(args.next().ok_or("Enter dB")?.parse()?),
            "input" => Command::Input(args.next().ok_or("Enter input ID")?),
            _ => return Err("Unknown command".into()),
        })?,
    };
    println!("{}", serde_json::to_string(&state)?);
    Ok(())
}
