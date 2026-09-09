//! One-way IR device controls. All filesystem and transmitter I/O stays on the worker.
use super::{Command, Details, Event, Work};
use couch_model::{commands::Function, Action, Integration};
use couch_webos::Button;
use std::sync::atomic::AtomicU64;

fn function(action: &Command) -> Result<Option<String>, String> {
    Ok(Some(match action {
        Command::Retry => return Ok(None),
        Command::IrFunction(name) if Function::parse(name).is_some() => name.clone(),
        Command::Key(key) => match key {
            Button::Up => "up",
            Button::Down => "down",
            Button::Left => "left",
            Button::Right => "right",
            Button::Enter => "ok",
            Button::Back => "back",
            Button::Home => "home",
            Button::Menu => "menu",
            Button::Red => "red",
            Button::Green => "green",
            Button::Blue => "blue",
            Button::Yellow => "yellow",
            _ => return Err("This key has no infrared function".into()),
        }
        .into(),
        Command::Volume(true) => "volume-up".into(),
        Command::Volume(false) => "volume-down".into(),
        Command::Channel(true) => "channel-up".into(),
        Command::Channel(false) => "channel-down".into(),
        Command::Mute(_) | Command::ToggleMute => "mute".into(),
        Command::Power => "toggle".into(),
        Command::Wake => "power-on".into(),
        Command::Play(true) => "play".into(),
        Command::Play(false) => "pause".into(),
        Command::Next(true) => "next".into(),
        Command::Next(false) => "previous".into(),
        Command::Stop => "stop".into(),
        Command::Rewind(true) => "fast-forward".into(),
        Command::Rewind(false) => "rewind".into(),
        _ => return Err("This control is not available for an infrared device".into()),
    }))
}

pub(super) fn run(work: &Work, active: &AtomicU64) -> Result<Option<Event>, String> {
    if !super::android::current(work, active) {
        return Ok(None);
    }
    let id = work
        .connection
        .strip_prefix("ir:")
        .ok_or("Missing IR device")?;
    let config = crate::connections::config().ok_or("Configuration unavailable")?;
    let (_, device) = config
        .devices()
        .find(|(_, d)| d.id.as_str() == id)
        .ok_or("IR device was removed")?;
    let integration = config
        .resolve_integration(&device.integration)
        .ok_or("IR connection was removed")?;
    let Integration::Ir { codeset } = &integration else {
        return Err("Selected device is not infrared".into());
    };
    let codes =
        couch_ir::codeset::load(&crate::home::path("ir"), codeset).map_err(|e| e.to_string())?;
    let choices = codes
        .entries
        .iter()
        .filter_map(|entry| {
            let f = Function::parse(&entry.button)?;
            f.supports(&integration).then(|| {
                (
                    format!("ir:{}", f.id()),
                    entry.button.replace('-', " "),
                    "Send infrared command".into(),
                )
            })
        })
        .collect();
    let command = function(&work.action)?;
    if !super::android::current(work, active) {
        return Ok(None);
    }
    let status = if let Some(command) = command {
        crate::activity_buttons::execute(
            &config,
            &Action::new(device.id.clone(), command),
            &mut Default::default(),
            &mut Default::default(),
            &mut Default::default(),
        )?;
        "IR command sent · No device feedback"
    } else {
        "Infrared · No device feedback"
    };
    Ok(Some(Event {
        generation: work.generation,
        status: Ok(status.into()),
        details: Some(Details {
            source: "Infrared controls".into(),
            choices,
            ..Details::default()
        }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_commands_use_explicit_ir_functions() {
        assert_eq!(
            function(&Command::Power).unwrap().as_deref(),
            Some("toggle")
        );
        assert_eq!(
            function(&Command::Wake).unwrap().as_deref(),
            Some("power-on")
        );
        assert_eq!(
            function(&Command::Key(Button::Enter)).unwrap().as_deref(),
            Some("ok")
        );
        assert_eq!(
            function(&Command::Rewind(true)).unwrap().as_deref(),
            Some("fast-forward")
        );
        assert!(function(&Command::App("fake".into())).is_err());
        assert!(function(&Command::Retry).unwrap().is_none());
    }
}
