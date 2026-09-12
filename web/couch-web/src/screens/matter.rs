//! Matter: pair devices shared from another ecosystem onto the remote's own fabric.
use crate::{api, App};
use couch_model::Connection;
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

fn fail(app: App, message: RwSignal<String>, error: api::ApiError) {
    if error.unauthorized {
        app.paired.set(Some(false));
    }
    message.set(error.message);
}

pub(super) fn setup(app: App, connection: &Connection) -> AnyView {
    let base = StoredValue::new(format!("/api/connections/{}/matter", connection.id));
    let code = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let devices = RwSignal::new(Vec::<Value>::new());
    let refresh = move || {
        spawn_local(async move {
            match api::ha("GET", &format!("{}/devices", base.get_value()), None).await {
                Ok(v) => devices.set(v.as_array().cloned().unwrap_or_default()),
                Err(e) => fail(app, message, e),
            }
        });
    };
    refresh();
    let pair = move |_| {
        if busy.get_untracked() {
            return;
        }
        let data =
            json!({"code": code.get_untracked().trim(), "name": name.get_untracked().trim()});
        busy.set(true);
        message.set(
            "Searching for the device on your network and pairing. This can take up to a minute."
                .into(),
        );
        spawn_local(async move {
            match api::ha(
                "POST",
                &format!("{}/commission", base.get_value()),
                Some(data),
            )
            .await
            {
                Ok(node) => {
                    let switchable = node["endpoints"]
                        .as_array()
                        .map(|e| e.iter().filter(|e| e["on_off"] == true).count())
                        .unwrap_or(0);
                    message.set(if switchable == 0 {
                        format!(
                            "Paired {}, but it has no on/off control Couch can use yet.",
                            node["name"].as_str().unwrap_or("")
                        )
                    } else {
                        format!(
                            "Paired {}. Add it to a room from Rooms & devices.",
                            node["name"].as_str().unwrap_or("")
                        )
                    });
                    code.set(String::new());
                    name.set(String::new());
                    refresh();
                }
                Err(e) => fail(app, message, e),
            }
            busy.set(false);
        });
    };
    let forget = move |node_id: u64| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        message.set("Asking the device to forget this remote…".into());
        spawn_local(async move {
            match api::ha("DELETE", &format!("{}/devices/{node_id}", base.get_value()), None).await {
                Ok(v) => message.set(if v["fabric_released"] == true {
                    "Removed. The device no longer lists this remote.".into()
                } else {
                    "Removed here. The device did not answer, so it still lists this remote until it is reset.".into()
                }),
                Err(e) => fail(app, message, e),
            }
            refresh();
            busy.set(false);
        });
    };
    view! {
        <section class="card matter-connection"><h2>"Matter"</h2>
        <p>"Couch runs its own Matter controller. In the app that set up a device (Apple Home, Google Home, Alexa or Home Assistant), choose to pair it with another app or service, then enter the pairing code it shows here before it expires. The device stays in that app as well. Pairing works over Wi-Fi or Ethernet; a new device still in its box must be set up in its own app first."</p>
        <label class="field">"Pairing code"<input type="text" inputmode="numeric" placeholder="3497-011-2332" prop:value=move || code.get() disabled=move || busy.get() on:input=move |e| code.set(event_target_value(&e))/></label>
        <label class="field">"Device name"<input type="text" placeholder="Reading lamp" prop:value=move || name.get() disabled=move || busy.get() on:input=move |e| name.set(event_target_value(&e))/></label>
        <div class="actions"><button class="primary" disabled=move || busy.get() on:click=pair>"Pair device"</button></div>
        <p role="status">{move || message.get()}</p>
        <h3>"Paired devices"</h3>
        {move || { let list = devices.get(); if list.is_empty() { view!{<p class="dim">"No devices paired with this remote yet."</p>}.into_any() } else { list.into_iter().map(|d| {
            let node_id = d["node_id"].as_u64().unwrap_or(0);
            let title = d["name"].as_str().unwrap_or("").to_string();
            let product = format!("{} {}", d["vendor_name"].as_str().unwrap_or(""), d["product_name"].as_str().unwrap_or("")).trim().to_string();
            let switchable = d["endpoints"].as_array().map(|e| e.iter().filter(|e| e["on_off"] == true).count()).unwrap_or(0);
            let detail = format!("{} · node {node_id} · {switchable} on/off control{}", if product.is_empty() { "Matter device".to_string() } else { product }, if switchable == 1 { "" } else { "s" });
            view!{<div class="card discovered-device"><strong>{title}</strong><p class="dim">{detail}</p>
                <button class="ghost" disabled=move || busy.get() on:click=move |_| forget(node_id)>"Forget device"</button></div>}
        }).collect_view().into_any() } }}
        </section>
    }.into_any()
}
