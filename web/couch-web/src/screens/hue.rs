use crate::{api, App};
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

fn fail(app: App, message: RwSignal<String>, error: api::ApiError) {
    if error.unauthorized {
        app.paired.set(Some(false));
    }
    message.set(error.message);
}
pub fn setup(app: App) -> AnyView {
    let url = RwSignal::new(String::new());

    let token_set = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let lights = RwSignal::new(Vec::<Value>::new());
    spawn_local(async move {
        match api::ha("GET", "/api/hue/connection", None).await {
            Ok(s) => {
                url.set(s["url"].as_str().unwrap_or("").into());
                token_set.set(s["token_set"].as_bool().unwrap_or(false));
            }
            Err(e) => fail(app, message, e),
        }
    });
    let save = move |_| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        message.set("Pairing with the bridge…".into());
        let data = json!({"url":url.get_untracked().trim()});
        spawn_local(async move {
            match api::ha("PUT", "/api/hue/connection", Some(data)).await {
                Ok(result) => {
                    token_set.set(true);
                    lights.set(result["lights"].as_array().cloned().unwrap_or_default());
                    message.set(
                        "Connected and saved. Choose a light below to add it to a room.".into(),
                    );
                }
                Err(e) => fail(app, message, e),
            }
            busy.set(false);
        });
    };
    let discover = move |_| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        message.set("Finding lights…".into());
        spawn_local(async move {
            match api::ha("GET", "/api/hue/lights", None).await {
                Ok(result) => {
                    let list = result.as_array().cloned().unwrap_or_default();
                    message.set(format!("Found {} lights", list.len()));
                    lights.set(list);
                }
                Err(e) => fail(app, message, e),
            }
            busy.set(false);
        });
    };
    view! {
        <section class="card hue-connection"><h2>"Philips Hue"</h2>
        <p>"Control Hue lights directly on your local network. Pair your bridge, then add its lights to rooms."</p>
        <label class="field">"Hue bridge address"<input type="text" placeholder="192.168.1.157" prop:value=move || url.get() disabled=move || busy.get() on:input=move |e| url.set(event_target_value(&e))/></label>
        <p class="dim">"Press the round link button on your bridge, then Pair bridge. Couch trusts this bridge on first pairing and pins its HTTPS certificate. Pair again after a certificate change."</p>
        <div class="actions"><button class="primary" disabled=move || busy.get() on:click=save>"Pair bridge"</button>
        <button class="ghost" disabled=move || busy.get() || !token_set.get() on:click=discover>"Find lights"</button></div>
        <p role="status">{move || message.get()}</p>
        {move || lights.get().into_iter().map(|light| discovered(app,light)).collect_view()}
        </section>
    }.into_any()
}
fn discovered(app: App, light: Value) -> AnyView {
    let id = light["entity_id"].as_str().unwrap_or("").to_string();
    let name = light["name"].as_str().unwrap_or(&id).to_string();
    let room = RwSignal::new(String::new());
    let rooms: Vec<_> = app
        .config
        .get_untracked()
        .map(|c| {
            c.rooms
                .into_iter()
                .map(|r| (r.id.to_string(), r.name))
                .collect()
        })
        .unwrap_or_default();
    let already = {
        let id = id.clone();
        move || {
            app.config.get().is_some_and(|c| c.devices().any(|(_,d)| matches!(&d.integration,couch_model::Integration::Hue{light_id} if light_id == &id)))
        }
    };
    let add_id = id.clone();
    let add_name = name.clone();
    view! {
        <div class="card"><strong>{name}</strong><p class="dim">{id}</p>
        {controls(app,light.clone())}
        <p>{if light["on"].is_null() {"Unavailable"} else if light["dimmable"] == true {"On/off and brightness"} else {"On/off"}}</p>
        <label class="field">"Add to room"<select aria-label="Add to room" prop:value=move || room.get() on:change=move |e| room.set(event_target_value(&e))><option value="">"Choose a room"</option>{rooms.into_iter().map(|(id,name)| view!{<option value=id>{name}</option>}).collect_view()}</select></label>
        <button class="ghost" disabled=move || app.busy.get() || room.get().is_empty() || already() on:click=move |_| {
            app.run(api::post(format!("/api/rooms/{}/devices",room.get_untracked()),json!({"name":add_name,"kind":"light","integration":{"via":"hue","light_id":add_id}})));
        }>"Add light"</button>
        </div>
    }.into_any()
}

fn controls(app: App, initial: Value) -> AnyView {
    let light = RwSignal::new(initial);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let level = RwSignal::new("50".to_string());
    let perform = move |action: Option<Value>| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        let id = light.get_untracked()["entity_id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        spawn_local(async move {
            if let Some(action) = action {
                if let Err(e) = api::ha(
                    "POST",
                    &format!("/api/hue/lights/{id}/command"),
                    Some(action),
                )
                .await
                {
                    fail(app, message, e);
                    busy.set(false);
                    return;
                }
                message.set("Request accepted. Refresh to check the latest state.".into());
            } else {
                message.set(String::new());
            }
            match api::ha("GET", &format!("/api/hue/lights/{id}"), None).await {
                Ok(value) => light.set(value),
                Err(e) => fail(app, message, e),
            }
            busy.set(false);
        });
    };
    view! {
        <p>{move || { let s = light.get(); match s["on"].as_bool() { Some(true) => match s["brightness_percent"].as_u64() {Some(n) => format!("On · {n}%"),None => "On".into()},Some(false) => "Off".into(),None => "Unavailable".into()} }}</p>
        <div class="actions"><button class="ghost" disabled=move || busy.get() || light.get()["on"].is_null() on:click=move |_| perform(Some(json!({"action":"on"})))>"Turn on"</button>
        <button class="ghost" disabled=move || busy.get() || light.get()["on"].is_null() on:click=move |_| perform(Some(json!({"action":"off"})))>"Turn off"</button>
        <button class="ghost" disabled=move || busy.get() on:click=move |_| perform(None)>"Refresh state"</button></div>
        <Show when=move || light.get()["dimmable"] == true>
            <label class="field">"Brightness (%)"<input type="number" min="0" max="100" prop:value=move || level.get() disabled=move || busy.get() on:input=move |e| level.set(event_target_value(&e))/></label>
            <button class="ghost" disabled=move || busy.get() || light.get()["on"].is_null() on:click=move |_| {
                match level.get_untracked().parse::<u8>() { Ok(p) if p <= 100 => perform(Some(json!({"action":"brightness","brightness":p}))), _ => message.set("Enter brightness from 0 to 100".into()) }
            }>"Apply brightness"</button>
        </Show>
        <p role="status">{move || message.get()}</p>
    }.into_any()
}
