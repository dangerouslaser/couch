use couch_tizen::{rest, Client, Key, Settings};
use std::{env, path::Path, process::ExitCode, time::Duration};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!("couch-tizen info <TV_IP>\ncouch-tizen discover\ncouch-tizen pair <TV_IP> <settings.json> [--legacy]\ncouch-tizen <settings.json> status|key KEY_NAME|apps|launch APP_ID|power|wake\ncouch-tizen wake <MAC> <IPv4 broadcast>\nPairing shows an Allow prompt on the TV. --legacy uses unencrypted ws://TV_IP:8001/ for 2016 models that issue no token.");
        return Ok(());
    }
    let get = |i: usize| args.get(i).map(String::as_str).ok_or("Missing argument");
    match args[0].as_str() {
        "info" => {
            let info = rest::device_info(get(1)?.parse()?, Duration::from_secs(4))?;
            println!("{}", serde_json::to_string_pretty(&info)?);
            return Ok(());
        }
        "discover" => {
            for address in couch_tizen::discovery::search(Duration::from_secs(3))? {
                match rest::device_info(address, Duration::from_secs(2)) {
                    Ok(info) => println!("{address}\t{}\t{}", info.model, info.name),
                    Err(e) => println!("{address}\t(no REST answer: {e})"),
                }
            }
            return Ok(());
        }
        "pair" => {
            let address: std::net::IpAddr = get(1)?.parse()?;
            let legacy = args.get(3).map(String::as_str) == Some("--legacy");
            let info = rest::device_info(address, Duration::from_secs(4)).ok();
            let secure = !legacy && info.as_ref().is_none_or(|i| i.token_auth);
            eprintln!("Allow Couch on your TV when it asks (up to 60 seconds).");
            let (_, mut settings) = Client::pair(&couch_tizen::base_url(address, secure))?;
            if let Some(info) = info {
                settings.mac = info.mac;
                settings.frame_tv = info.frame_tv;
                settings.model = info.model;
                settings.name = info.name;
            }
            settings.save(Path::new(get(2)?))?;
            println!("Paired; credentials saved privately.");
            return Ok(());
        }
        "wake" => {
            couch_tizen::wake(get(1)?, get(2)?.parse()?)?;
            println!("Wake packet sent; this does not confirm the TV is awake.");
            return Ok(());
        }
        _ => {}
    }
    let settings = Settings::load(Path::new(&args[0]))?;
    match get(1)? {
        "wake" => {
            let mac = settings
                .mac
                .as_deref()
                .ok_or("The TV did not report a MAC while pairing")?;
            couch_tizen::wake(mac, "255.255.255.255".parse()?)?;
            println!("Wake packet sent to {mac}; this does not confirm the TV is awake.");
            return Ok(());
        }
        "status" => {
            let info = rest::device_info(settings.address()?, Duration::from_secs(4))?;
            let mut client = Client::connect(&settings)?;
            client.idle()?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"device":info,"remote":"connected"})
                )?
            );
            return Ok(());
        }
        _ => {}
    }
    let mut client = Client::connect(&settings)?;
    match get(1)? {
        "key" => {
            let wanted = get(2)?.to_ascii_uppercase();
            let key = [
                Key::Up,
                Key::Down,
                Key::Left,
                Key::Right,
                Key::Enter,
                Key::Return,
                Key::Exit,
                Key::Home,
                Key::Menu,
                Key::Info,
                Key::Guide,
                Key::Source,
                Key::Tools,
                Key::ChannelUp,
                Key::ChannelDown,
                Key::VolumeUp,
                Key::VolumeDown,
                Key::Mute,
                Key::Power,
                Key::Play,
                Key::Pause,
                Key::Stop,
                Key::Rewind,
                Key::FastForward,
                Key::Red,
                Key::Green,
                Key::Yellow,
                Key::Blue,
                Key::Tv,
                Key::Hdmi,
                Key::Hdmi1,
                Key::Hdmi2,
                Key::Hdmi3,
                Key::Hdmi4,
            ]
            .into_iter()
            .find(|k| k.name() == wanted)
            .ok_or("Unknown key; use a KEY_* name")?;
            client.key(key)?;
            println!("Key sent (the remote channel has no acknowledgement).");
        }
        "apps" => {
            for app in client.apps()? {
                println!("{}\t{}\t{}", app.id, app.app_type, app.name);
            }
        }
        "launch" => {
            client.launch_app(get(2)?)?;
            println!("App launch requested.");
        }
        "power" => {
            client.power_toggle(settings.frame_tv)?;
            println!("Power key sent; Tizen toggles power rather than reporting it.");
        }
        _ => return Err("Unknown command; use --help".into()),
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("couch-tizen: {e}");
            ExitCode::FAILURE
        }
    }
}
