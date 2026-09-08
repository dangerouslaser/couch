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
                Ok(_result) => {
                    token_set.set(true);
                    if !app
                        .config
                        .get_untracked()
                        .is_some_and(|c| c.connections.iter().any(|c| c.provider.kind() == "hue"))
                    {
                        app.run(api::post(
                            "/api/connections",
                            json!({"name":"Philips Hue","provider":{"kind":"hue"}}),
                        ));
                    }

                    message.set("Connected and saved. Add devices from Rooms & devices.".into());
                }
                Err(e) => fail(app, message, e),
            }
            busy.set(false);
        });
    };
    view! {
        <section class="card hue-connection"><h2>"Philips Hue"</h2>
        <p>"Pair the bridge used to reach your Hue lights, rooms and scenes. Add lights, Hue rooms and scenes in Rooms & devices."</p>
        <label class="field">"Hue bridge address"<input type="text" placeholder="192.168.1.157" prop:value=move || url.get() disabled=move || busy.get() on:input=move |e| url.set(event_target_value(&e))/></label>
        <p class="dim">"Press the round link button on your bridge, then Pair bridge. Couch trusts this bridge on first pairing and pins its HTTPS certificate. Pair again after a certificate change."</p>
        <div class="actions"><button class="primary" disabled=move || busy.get() on:click=save>"Pair bridge"</button>
        <Show when=move || token_set.get() && !app.config.get().is_some_and(|c|c.connections.iter().any(|c|c.provider.kind()=="hue"))><button class="ghost" disabled=move ||busy.get() on:click=move |_|app.run(api::post("/api/connections",json!({"name":"Philips Hue","provider":{"kind":"hue"}})))>"Use saved connection"</button></Show>
        </div>
        <p role="status">{move || message.get()}</p>

        </section>
    }.into_any()
}
pub(super) fn controls(app: App, initial: Value) -> AnyView {
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
