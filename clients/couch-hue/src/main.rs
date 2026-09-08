use couch_hue::{settings::Settings, Command, Hue};
fn main() {
    if let Err(e) = run() {
        eprintln!("couch-hue: {e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut file = "/opt/couch/hue-connection.json".to_string();
    let mut command = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--settings" {
            file = args.next().ok_or("Missing settings path")?
        } else {
            command.push(arg)
        }
    }
    let args: Vec<_> = command.iter().map(String::as_str).collect();
    let path = std::path::Path::new(&file);
    if let ["pair", address] = args.as_slice() {
        let s = Hue::pair(address)?;
        s.save(path)?;
        println!("Hue bridge paired; credentials saved privately");
        return Ok(());
    }
    if args.is_empty() || args == ["--help"] {
        println!("couch-hue [--settings PATH] pair BRIDGE_IP | lights | state UUID | on UUID | off UUID | brightness UUID PERCENT");
        return Ok(());
    }
    let c = Settings::load(path)?.client()?;
    match args.as_slice() {
        ["lights"] => println!("{}", serde_json::to_string_pretty(&c.lights()?)?),
        ["state", id] => println!("{}", serde_json::to_string_pretty(&c.light(id)?)?),
        ["on", id] => c.command(id, Command::On)?,
        ["off", id] => c.command(id, Command::Off)?,
        ["brightness", id, p] => c.command(id, Command::Brightness(p.parse()?))?,
        _ => return Err("Unknown command; use --help".into()),
    }
    Ok(())
}
