use couch_ha::{Command, HomeAssistant};
fn main() {
    if let Err(e) = run() {
        eprintln!("couch-ha: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut url = std::env::var("COUCH_HA_URL").ok();
    let mut token_file = "/opt/couch/ha-token".to_string();
    let mut command = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = Some(args.next().ok_or("--url needs a URL")?),
            "--token-file" => token_file = args.next().ok_or("--token-file needs a path")?,
            "--help" | "-h" => {
                println!("couch-ha --url http://homeassistant.local:8123 [--token-file PATH] COMMAND\n\nCommands: lights | state light.ID | on light.ID | off light.ID | brightness light.ID PERCENT\n\nURL defaults to COUCH_HA_URL. Token defaults to /opt/couch/ha-token (chmod 600).\nCommands change real lights. Discovery and state are read-only. HTTPS verifies certificates.");
                return Ok(());
            }
            _ => command.push(arg),
        }
    }
    let url = url.ok_or("Set --url or COUCH_HA_URL")?;
    let token = std::fs::read_to_string(token_file)
        .map_err(|_| "Cannot read the Home Assistant token file")?;
    let client = HomeAssistant::new(&url, token.trim())?;
    match command
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["lights"] => println!("{}", serde_json::to_string_pretty(&client.lights()?)?),
        ["state", id] => println!("{}", serde_json::to_string_pretty(&client.light(id)?)?),
        ["on", id] => {
            client.command(id, Command::On)?;
            println!("On request accepted");
        }
        ["off", id] => {
            client.command(id, Command::Off)?;
            println!("Off request accepted");
        }
        ["brightness", id, p] => {
            client.command(id, Command::Brightness(p.parse()?))?;
            println!("Brightness request accepted");
        }
        _ => return Err("Unknown command; use --help".into()),
    }
    Ok(())
}
