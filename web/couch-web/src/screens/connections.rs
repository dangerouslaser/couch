use crate::{api, ui, App};
use couch_model::{Config, Connection, Provider};
use leptos::prelude::*;
use serde_json::json;

pub fn screen(app: App, config: &Config) -> AnyView {
    let choice = RwSignal::new(String::new());
    let existing = config.connections.clone();
    let available: Vec<_> = [
        ("kodi", "Kodi"),
        ("home-assistant", "Home Assistant"),
        ("hue", "Philips Hue"),
        ("ir", "Infrared"),
    ]
    .into_iter()
    .filter(|(kind, _)| *kind == "kodi" || !existing.iter().any(|c| c.provider.kind() == *kind))
    .collect();
    view!{
        {ui::page_header(app,"Connections".into(),None)}
        <p class="lead">"Connections tell Couch how to reach your servers, bridges and infrared transmitter. Add devices to rooms in Rooms & devices."</p>
        <h2 class="section">"Saved connections"</h2>
        {config.connections.is_empty().then(||ui::empty("No connections yet. Add your first connection below."))}
        <div class="destination-grid">{existing.into_iter().map(|c|saved(app,c)).collect_view()}</div>
        <section class="creation"><h2>"Add a connection"</h2>
        <label class="field">"Connection type"<select aria-label="Connection type" prop:value=move || choice.get() on:change=move |e|choice.set(event_target_value(&e))><option value="">"Choose a type"</option>{available.into_iter().map(|(kind,label)|view!{<option value=kind>{label}</option>}).collect_view()}</select></label>
        {move || match choice.get().as_str(){"kodi"=>local_form(app,None,false),"ir"=>local_form(app,None,true),"home-assistant"=>super::home_assistant::setup(app),"hue"=>super::hue::setup(app),_=>view!{<p class="dim">"Kodi supports multiple players. One Home Assistant server, Hue bridge and infrared transmitter are supported."</p>}.into_any()}}
        </section>
    }.into_any()
}
fn saved(app: App, c: Connection) -> AnyView {
    let label = c.provider.label();
    let title = c.name.clone();
    let id = c.id.clone();
    let usage=app.config.get_untracked().map(|cfg|cfg.devices().filter(|(_,d)|matches!(&d.integration,couch_model::Integration::Connection{connection_id,..} if connection_id==&id)).count()).unwrap_or(0);
    let edit = match c.provider {
        Provider::Kodi { .. } => local_form(app, Some(c.clone()), false),
        Provider::Ir => local_form(app, Some(c.clone()), true),
        Provider::HomeAssistant => super::home_assistant::setup(app),
        Provider::Hue => super::hue::setup(app),
    };
    view!{<section class="card saved-connection"><h2>{title}</h2><p>{format!("{label} · {usage} assigned devices")}</p>
        <p class="dim">{match &c.provider{Provider::Kodi{host,port}=>format!("{host}:{port} · Saved address"),Provider::Ir=>"Built-in transmitter · Sending is currently unavailable on this device".into(),_=>"Credentials are kept privately on the remote".into()}}</p>
        <details><summary>"Connection settings"</summary>{edit}</details>
        <p class="dim">"Removing a connection requires removing its assigned devices first. Bridge credentials are retained for reconnecting."</p>
        {ui::danger_button("Remove connection",move ||app.run(api::delete(format!("/api/connections/{id}"))))}
    </section>}.into_any()
}
fn local_form(app: App, existing: Option<Connection>, infrared: bool) -> AnyView {
    let name = RwSignal::new(
        existing
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or(if infrared {
                "Infrared".into()
            } else {
                String::new()
            }),
    );
    let (initial_host, initial_port) = match existing.as_ref().map(|c| &c.provider) {
        Some(Provider::Kodi { host, port }) => (host.clone(), port.to_string()),
        _ => (String::new(), "9090".into()),
    };
    let host = RwSignal::new(initial_host);
    let port = RwSignal::new(initial_port);
    let error = RwSignal::new(String::new());
    view!{<form on:submit=move |e|{e.prevent_default();let name=name.get_untracked().trim().to_string();if name.is_empty(){error.set("Enter a connection name".into());return}
        let provider=if infrared{Provider::Ir}else{let Ok(port)=port.get_untracked().parse::<u16>()else{error.set("Enter a TCP port from 1 to 65535".into());return};if port==0 || host.get_untracked().trim().is_empty(){error.set("Enter a hostname and a TCP port from 1 to 65535".into());return}Provider::Kodi{host:host.get_untracked().trim().into(),port}};
        let body=json!({"name":name,"provider":provider});match &existing{Some(c)=>app.run(api::put(format!("/api/connections/{}",c.id),body)),None=>app.run(api::post("/api/connections",body))}
    }>
        {field("Connection name",name,"Living room Kodi")}
        {(!infrared).then(||view!{<p class="dim">"Enable remote control in Kodi. Saving an address does not test connectivity."</p>{field("Hostname or IP address",host,"kodi.local")}{field("TCP port",port,"9090")}})}
        {infrared.then(||view!{<p class="notice">"Use the remote’s built-in infrared transmitter. Configure each device’s codeset inside its room. IR sending and learning are not yet available on the current kernel."</p>})}
        <p role="alert">{move ||error.get()}</p><button class="primary" type="submit">"Save connection"</button>
    </form>}.into_any()
}
pub(super) fn field(
    label: &'static str,
    value: RwSignal<String>,
    placeholder: &'static str,
) -> AnyView {
    view!{<label class="field">{label}<input type="text" prop:value=move ||value.get() placeholder=placeholder on:input=move |e|value.set(event_target_value(&e))/></label>}.into_any()
}

pub(super) fn label(c: &Connection) -> String {
    if c.name == c.provider.label() {
        c.name.clone()
    } else {
        format!("{} · {}", c.name, c.provider.label())
    }
}
