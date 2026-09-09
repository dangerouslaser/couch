//! Shared IR catalog selection. Previewing and saving never transmit a command.
use crate::{api, App};
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

pub fn library(app: App, output: RwSignal<String>, power_only: bool) -> AnyView {
    let catalog = RwSignal::new(Vec::<Value>::new());
    let brand = RwSignal::new(String::new());
    let kind = RwSignal::new(String::new());
    let selected = RwSignal::new(String::new());
    let commands = RwSignal::new(Vec::<Value>::new());
    let busy = RwSignal::new(true);
    let message = RwSignal::new(String::new());
    let source = RwSignal::new(String::new());
    let import_text = RwSignal::new(String::new());
    let import_format = RwSignal::new("flipper".to_string());
    let import_name = RwSignal::new("Imported remote".to_string());
    spawn_local(async move {
        let result = api::ha("GET", "/api/ir/catalog", None).await;
        if busy.try_get_untracked().is_none() {
            return;
        }
        match result {
            Ok(v) => {
                catalog.set(v["codesets"].as_array().cloned().unwrap_or_default());
                source.set(format!(
                    "{} · {}",
                    v["source"]["name"].as_str().unwrap_or("IR library"),
                    v["source"]["license"]
                        .as_str()
                        .unwrap_or("See source license")
                ));
            }
            Err(e) => {
                if e.unauthorized {
                    app.paired.set(Some(false));
                }
                message.set(e.message);
            }
        }
        busy.set(false);
    });
    let fetch = move |path: String, body: Option<Value>| {
        busy.set(true);
        commands.set(Vec::new());
        message.set(String::new());
        spawn_local(async move {
            let result = api::ha(if body.is_some() { "POST" } else { "GET" }, &path, body).await;
            if busy.try_get_untracked().is_none() {
                return;
            }
            match result {
                Ok(v) => {
                    commands.set(v["commands"].as_array().cloned().unwrap_or_default());
                    message.set("Choose a function for each command you want to use. No command has been sent.".into());
                }
                Err(e) => {
                    if e.unauthorized {
                        app.paired.set(Some(false));
                    }
                    message.set(e.message);
                }
            }
            busy.set(false);
        });
    };
    view! {
        <div class="ir-library"><h4>"Choose remote codes"</h4>
            <p class="dim">{move ||source.get()}</p>
            <label class="field">"Brand"<select aria-label="IR brand" disabled=move ||busy.get() prop:value=move ||brand.get() on:change=move |e|{brand.set(event_target_value(&e));kind.set(String::new());selected.set(String::new());commands.set(Vec::new());}>
                <option value="">"Choose a brand"</option>{move ||catalog.get().iter().filter_map(|v|v["brand"].as_str()).map(str::to_string).collect::<std::collections::BTreeSet<_>>().into_iter().map(|s|view!{<option value=s.clone()>{s.clone()}</option>}).collect_view()}
            </select></label>
            <label class="field">"Device type"<select aria-label="IR library device type" prop:value=move ||kind.get() disabled=move ||busy.get()||brand.get().is_empty() on:change=move |e|{kind.set(event_target_value(&e));selected.set(String::new());commands.set(Vec::new());}>
                <option value="">"All device types"</option>{move ||catalog.get().iter().filter(|v|v["brand"].as_str()==Some(brand.get().as_str())).filter_map(|v|v["device_type"].as_str()).map(str::to_string).collect::<std::collections::BTreeSet<_>>().into_iter().map(|s|view!{<option value=s.clone()>{s.clone()}</option>}).collect_view()}
            </select></label>
            <label class="field">"Model / codeset"<select aria-label="IR model" disabled=move ||busy.get()||brand.get().is_empty() prop:value=move ||selected.get() on:change=move |e|{let id=event_target_value(&e);selected.set(id.clone());if !id.is_empty(){fetch(format!("/api/ir/catalog/{id}"),None);}}>
                <option value="">"Choose a model"</option>{move ||catalog.get().into_iter().filter(|v|v["brand"].as_str()==Some(brand.get().as_str())&&(kind.get().is_empty()||v["device_type"].as_str()==Some(kind.get().as_str()))).map(|v|{let id=v["id"].as_str().unwrap_or("").to_string();let label=format!("{} — {} supported commands",v["model"].as_str().unwrap_or(&id),v["supported_commands"]);view!{<option value=id>{label}</option>}}).collect_view()}
            </select></label>
            <details><summary>"Import your own remote codes"</summary>
                {super::connections::field("Import name",import_name,"Remote model")}
                <label class="field">"File format"<select aria-label="IR import format" on:change=move |e|import_format.set(event_target_value(&e))><option value="flipper">"Flipper .ir"</option><option value="couch">"Couch codeset"</option></select></label>
                <label class="field">"Paste file contents"<textarea aria-label="IR import contents" rows="5" maxlength="262144" prop:value=move ||import_text.get() on:input=move |e|import_text.set(event_target_value(&e)) /></label>
                <button type="button" disabled=move ||busy.get()||import_text.get().trim().is_empty() on:click=move |_|fetch("/api/ir/import".into(),Some(json!({"name":import_name.get_untracked(),"format":import_format.get_untracked(),"text":import_text.get_untracked()})))>"Preview imported commands"</button>
            </details>
            <p role="status">{move ||message.get()}</p>
            <div class="ir-command-list" aria-label="Available IR commands">{move ||commands.get().into_iter().enumerate().map(|(index,v)|{
                let name=v["name"].as_str().unwrap_or("Unnamed command").to_string();
                let supported=v["supported"]==true && v["code"].is_string();
                let code=v["code"].as_str().unwrap_or("").to_string();
                let reason=v["reason"].as_str().unwrap_or("Protocol is not supported by Couch").to_string();
                let label=format!("Assign {name} ({})",index+1);
                view!{<div class="ir-command"><strong>{name}</strong>{if supported {
                    view!{<label class="field">"Remote function"<select aria-label=label on:change=move |e|{let function=event_target_value(&e);if !function.is_empty(){output.update(|text|assign(text,&function,&code));}}><option value="">"Choose a function"</option>{(if power_only {vec!["power","power-on","power-off"]} else {vec!["toggle","power-on","power-off","up","down","left","right","ok","back","home","menu","volume-up","volume-down","mute","channel-up","channel-down","play","pause","play-pause","stop","next","previous","rewind","fast-forward","red","green","yellow","blue"]}).into_iter().map(|s|view!{<option value=s>{s.replace('-'," ")}</option>}).collect_view()}</select></label>}.into_any()
                }else{view!{<p class="dim">{format!("Unavailable: {reason}")}</p>}.into_any()}}</div>}
            }).collect_view()}</div>
            <p class="dim">"Library codes are candidates, not proof of compatibility. Saving does not send IR. Discrete power on/off must use their own codes; a toggle is never substituted."</p>
        </div>
    }.into_any()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}

fn assign(text: &mut String, function: &str, code: &str) {
    let fields: Vec<_> = code.split_whitespace().collect();
    if fields.len() < 2 {
        return;
    }
    let mut lines: Vec<String> = text
        .lines()
        .filter(|line| line.split_whitespace().next() != Some(function))
        .map(str::to_string)
        .collect();
    lines.push(format!("{function} {}", fields[1..].join(" ")));
    *text = lines.join("\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifiers_cannot_change_request_paths() {
        assert!(valid_id("living-room_tv2"));
        for id in ["", "../codes", "a/b", "a?b", "UPPER", "-leading"] {
            assert!(!valid_id(id));
        }
    }
    #[test]
    fn raw_assignment_preserves_pulses() {
        let mut text = String::new();
        assign(&mut text, "power-off", "Off raw 38000 9000,4500,560,560");
        assert_eq!(text, "power-off raw 38000 9000,4500,560,560");
    }
    #[test]
    fn assignment_replaces_only_requested_function() {
        let mut text = "power nec 4 8\npower-on nec 4 9".to_string();
        assign(&mut text, "power", "source nec 4 10");
        assert_eq!(text, "power-on nec 4 9\npower nec 4 10");
        assign(&mut text, "power-off", "invalid");
        assert!(!text.contains("power-off"));
    }
}

pub fn device_setup(
    app: App,
    connection: couch_model::Id,
    room: couch_model::Id,
    existing: Option<couch_model::Device>,
) -> AnyView {
    let name = RwSignal::new(
        existing
            .as_ref()
            .map(|d| d.name.clone())
            .unwrap_or_default(),
    );
    let kind = RwSignal::new(
        existing
            .as_ref()
            .map(|d| d.kind.name().to_string())
            .unwrap_or("tv".into()),
    );
    let codeset = RwSignal::new(
        existing
            .as_ref()
            .and_then(|d| match &d.integration {
                couch_model::Integration::Connection { resource_id, .. } => {
                    Some(resource_id.clone())
                }
                couch_model::Integration::Ir { codeset } => Some(codeset.clone()),
                _ => None,
            })
            .unwrap_or_default(),
    );
    let text = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let saved = StoredValue::new(existing);

    if !codeset.get_untracked().is_empty() {
        busy.set(true);
        spawn_local(async move {
            let result = api::ha(
                "GET",
                &format!("/api/ir/codesets/{}", codeset.get_untracked()),
                None,
            )
            .await;
            if busy.try_get_untracked().is_none() {
                return;
            }
            match result {
                Ok(v) => text.set(v["text"].as_str().unwrap_or("").into()),
                Err(e) => message.set(e.message),
            }
            busy.set(false);
        });
    }
    let connection = StoredValue::new(connection);
    let room = StoredValue::new(room);
    view!{<section class="card"><h3>{if saved.get_value().is_some(){"Infrared commands"}else{"Add an infrared device"}}</h3>
        {super::connections::field("IR device name",name,"Living room TV")}
        <label class="field">"Device type"<select aria-label="IR device kind" prop:value=move ||kind.get() on:change=move |e|kind.set(event_target_value(&e))><option value="tv">"TV"</option><option value="speaker">"Speaker / receiver"</option><option value="media-player">"Media player"</option><option value="other">"Other"</option></select></label>
        {library(app,text,false)}
        {super::connections::field("Saved codeset ID",codeset,"living-room-tv")}
        <p class="dim">"Use a unique ID for this device: lowercase letters, numbers, hyphens or underscores. Devices sharing an ID share the same commands."</p>
        <label class="field">"Assigned commands"<textarea aria-label="Assigned IR commands" rows="6" maxlength="262144" prop:value=move ||text.get() on:input=move |e|text.set(event_target_value(&e)) /></label>
        <button type="button" class="primary" disabled=move ||busy.get()||name.get().trim().is_empty()||!valid_id(codeset.get().trim())||text.get().trim().is_empty() on:click=move |_|{
            busy.set(true);message.set(String::new());
            let id=codeset.get_untracked().trim().to_string();let title=name.get_untracked().trim().to_string();let next_kind=kind.get_untracked();
            spawn_local(async move {
                let response=api::ha("PUT",&format!("/api/ir/codesets/{id}"),Some(json!({"text":text.get_untracked()}))).await;
                if busy.try_get_untracked().is_none(){return;}
                match response {
                    Ok(_)=>{let integration=json!({"via":"connection","connection_id":connection.get_value(),"resource_id":id});
                        if let Some(device)=saved.get_value(){let mut value=serde_json::to_value(&device).unwrap();value["name"]=json!(title);value["kind"]=json!(next_kind);value["integration"]=integration;app.run(api::put(format!("/api/rooms/{}/devices/{}",room.get_value(),device.id),value));}
                        else {app.run(api::post(format!("/api/rooms/{}/devices",room.get_value()),json!({"name":title,"kind":next_kind,"integration":integration})));}
                    }
                    Err(e)=>{if e.unauthorized{app.paired.set(Some(false));}message.set(e.message);}
                }
                busy.set(false);
            });
        }>{if saved.get_value().is_some(){"Save IR commands"}else{"Save and add to room"}}</button><p role="status">{move ||message.get()}</p>
    </section>}.into_any()
}
