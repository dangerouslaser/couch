use couch_webos::{Button, Client, Playback, Settings};
use std::{env, path::Path, process::ExitCode, time::Duration};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!("couch-webos pair <wss://TV_IP:3001/> <settings.json>\ncouch-webos <settings.json> status|volume [0..100]|mute on|off|inputs|input ID|apps|launch ID|button NAME|play|pause|stop|rewind|forward|off|watch\ncouch-webos wake <MAC> <IPv4 broadcast>\nPairing shows an approval prompt on the TV. Plain ws://TV_IP:3000/ is explicit opt-in for older TVs.");
        return Ok(());
    }
    let get = |i: usize| args.get(i).map(String::as_str).ok_or("Missing argument");
    if args[0] == "pair" {
        if args.len() != 3 {
            return Err("pair requires URL and settings path".into());
        }
        eprintln!("Accept the Couch pairing request on your TV (up to 60 seconds).");
        let (_, settings) = Client::pair(get(1)?)?;
        settings.save(Path::new(get(2)?))?;
        println!("Paired; credentials saved privately.");
        return Ok(());
    }
    if args[0] == "wake" {
        if args.len() != 3 {
            return Err("wake requires MAC and IPv4 broadcast".into());
        }
        couch_webos::wake(get(1)?, get(2)?.parse()?)?;
        println!("Wake packet sent; this does not confirm the TV is awake.");
        return Ok(());
    }
    let settings = Settings::load(Path::new(&args[0]))?;
    let mut client = Client::connect(&settings)?;
    let value = match get(1)? {
        "status" => Some(
            serde_json::json!({"power":client.power_state()?,"volume":client.volume()?,"app":client.foreground_app()?}),
        ),
        "volume" => {
            if let Some(v) = args.get(2) {
                client.set_volume(v.parse()?)?;
                None
            } else {
                Some(client.volume()?)
            }
        }
        "mute" => {
            client.mute(match get(2)? {
                "on" => true,
                "off" => false,
                _ => return Err("mute accepts on or off".into()),
            })?;
            None
        }
        "inputs" => Some(client.inputs()?),
        "input" => {
            client.select_input(get(2)?)?;
            None
        }
        "apps" => Some(client.apps()?),
        "launch" => {
            client.launch_app(get(2)?)?;
            None
        }
        "off" => {
            client.power_off()?;
            None
        }
        "play" | "pause" | "stop" | "rewind" | "forward" => {
            client.playback(match get(1)? {
                "play" => Playback::Play,
                "pause" => Playback::Pause,
                "stop" => Playback::Stop,
                "rewind" => Playback::Rewind,
                _ => Playback::FastForward,
            })?;
            None
        }
        "button" => {
            client.button(match get(2)?.to_ascii_uppercase().as_str() {
                "UP" => Button::Up,
                "DOWN" => Button::Down,
                "LEFT" => Button::Left,
                "RIGHT" => Button::Right,
                "OK" | "ENTER" => Button::Enter,
                "BACK" => Button::Back,
                "HOME" => Button::Home,
                "MENU" => Button::Menu,
                "INFO" => Button::Info,
                "EXIT" => Button::Exit,
                "RED" => Button::Red,
                "GREEN" => Button::Green,
                "YELLOW" => Button::Yellow,
                "BLUE" => Button::Blue,
                _ => return Err("Unknown button".into()),
            })?;
            println!("Button sent (the pointer protocol has no acknowledgement).");
            return Ok(());
        }
        "watch" => {
            for uri in [
                "ssap://audio/getVolume",
                "ssap://com.webos.applicationManager/getForegroundAppInfo",
                "ssap://com.webos.service.tvpower/power/getPowerState",
            ] {
                let update = client.subscribe(uri)?;
                println!("{} {}", update.subscription, update.payload)
            }
            loop {
                if let Some(update) = client.next_update(Duration::from_secs(30))? {
                    println!("{} {}", update.subscription, update.payload)
                }
            }
        }
        _ => return Err("Unknown command; use --help".into()),
    };
    if let Some(v) = value {
        println!("{}", serde_json::to_string_pretty(&v)?)
    } else {
        println!("Command acknowledged.")
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("couch-webos: {e}");
            ExitCode::FAILURE
        }
    }
}
