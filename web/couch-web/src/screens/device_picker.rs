//! Room-scoped discovery and assignment. Connection credentials never appear here.
use crate::{api, route::Route, App};
use couch_model::{Config, Connection, Device, Id, Integration, Provider};
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

pub fn picker(app: App, config: &Config, room: &Id) -> AnyView {
    let connections = config.connections.clone();
    let room = room.clone();
    if connections.is_empty() {
        return view!{<section class="creation"><h2>"Add devices"</h2><p>"Add a connection first, then choose its devices here."</p><button class="primary" on:click=move |_|app.go(Route::Connections)>"Set up a connection"</button></section>}.into_any();
    }
    if !connections
        .iter()
        .any(|c| c.id.as_str() == app.device_source.get_untracked())
    {
        app.device_source.set(if connections.len() == 1 {
            connections[0].id.to_string()
        } else {
            String::new()
        });
    }
    let options = connections.clone();
    view!{<section class="creation device-picker"><h2>"Add to this room"</h2><p class="dim">"Choose a connection, then add lights, room controls or scenes."</p>
        <label class="field">"From connection"<select aria-label="From connection" prop:value=move ||app.device_source.get() on:change=move |e|{app.device_filter.set(String::new());app.device_source.set(event_target_value(&e));}><option value="">"Choose a connection"</option>{options.into_iter().map(|c|view!{<option value=c.id.to_string()>{super::connections::label(&c)}</option>}).collect_view()}</select></label>
        {move ||connections.iter().find(|c|c.id.as_str()==app.device_source.get()).map(|c|match c.provider{Provider::Hue|Provider::HomeAssistant=>discover(app,c.clone(),room.clone()),_=>manual(app,c.clone(),room.clone())})}
    </section>}.into_any()
}
fn assigned(app: App, connection: &Connection, resource: &str) -> Option<String> {
    app.config.get_untracked().and_then(|cfg|cfg.devices().find_map(|(r,d)|{
        let same=matches!(&d.integration,Integration::Connection{connection_id,resource_id} if connection_id==&connection.id && resource_id==resource)
            || match cfg.resolve_integration(&d.integration){Some(Integration::Hue{light_id})=>connection.provider==Provider::Hue && light_id==format!("{}/{resource}",connection.id),Some(Integration::HomeAssistant{entity_id})=>connection.provider==Provider::HomeAssistant && entity_id==format!("{}/{resource}",connection.id),Some(Integration::Kodi{host,port})=>connection.provider==Provider::Kodi{host,port},_=>false};
        same.then(||r.name.clone())
    }))
}
fn discover(app: App, connection: Connection, room: Id) -> AnyView {
    let list = RwSignal::new(Vec::<Value>::new());
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let prefix = if connection.provider == Provider::Hue {
        "hue"
    } else {
        "ha"
    };
    let base=StoredValue::new(format!("/api/connections/{}/{prefix}",connection.id));
    let category = if prefix == "hue" {
        app.hue_category
    } else {
        RwSignal::new("lights".to_string())
    };
    let fetch = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        message.set("Finding devices…".into());
        let category = category.get_untracked();
        spawn_local(async move {
            match api::ha("GET", &format!("{}/{category}",base.get_value()), None).await {
                Ok(v) => {
                    let values = v.as_array().cloned().unwrap_or_default();
                    message.set(format!(
                        "{} controls found. Choose the ones that belong in this room.",
                        values.len()
                    ));
                    list.set(values);
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
    fetch();
    view!{<p class="dim">{if prefix=="hue" {"Add lights, grouped room controls or scenes. Scenes go straight into this room’s Scenes button on the remote."} else {"Currently supports lights with on/off and brightness."}}</p>
        {(prefix=="hue").then(||view!{<label class="field">"Hue controls"<select aria-label="Hue controls" prop:value=move ||category.get() disabled=move ||busy.get() on:change=move |e|{category.set(event_target_value(&e));app.device_filter.set(String::new());app.hue_room_filter.set(String::new());list.set(Vec::new());fetch();}><option value="lights">"Lights"</option><option value="rooms">"Hue rooms"</option><option value="scenes">"Hue scenes"</option></select></label>})}
        <button class="ghost" disabled=move ||busy.get() on:click=move |_|fetch()>"Refresh devices"</button>
        {super::connections::field("Search devices",app.device_filter,"Filter by name")}
        {(prefix=="hue").then(||view!{<label class="field">"Hue room or zone"<select aria-label="Hue room or zone" prop:value=move ||app.hue_room_filter.get() on:change=move |e|app.hue_room_filter.set(event_target_value(&e))><option value="">"All bridge rooms and zones"</option>{move ||list.get().iter().filter_map(|v|v["room_name"].as_str()).filter(|s|!s.is_empty()).map(str::to_string).collect::<std::collections::BTreeSet<_>>().into_iter().map(|name|view!{<option value=name.clone()>{name.clone()}</option>}).collect_view()}</select></label>})}
        <p role="status">{move ||message.get()}</p>
        <div class="discovered-devices">{move ||list.get().into_iter().filter(|d|{
            let text=format!("{} {}",d["name"].as_str().unwrap_or(""),d["room_name"].as_str().unwrap_or("")).to_lowercase();
            app.device_filter.get().to_lowercase().split_whitespace().all(|word|text.contains(word))
                && (prefix!="hue" || app.hue_room_filter.get().is_empty() || d["room_name"].as_str()==Some(app.hue_room_filter.get().as_str()))
        }).map(|d| discovery_card(app,&connection,&room,d)).collect_view()}</div>
    }.into_any()
}
fn discovery_card(app: App, connection: &Connection, room: &Id, value: Value) -> AnyView {
    let id = value["entity_id"].as_str().unwrap_or("").to_string();
    let name = value["name"].as_str().unwrap_or(&id).to_string();
    let scene = value["resource_kind"] == "scene";
    let saved = if scene {
        app.config.get().and_then(|c| {
            c.scenes
                .iter()
                .find(|s| {
                    s.hue.as_ref().is_some_and(|h| {
                        h.connection_id == connection.id
                            && Some(h.scene_id.as_str()) == id.strip_prefix("scene:")
                    })
                })
                .cloned()
        })
    } else {
        None
    };
    let existing = if scene {
        None
    } else {
        assigned(app, connection, &id)
    };
    let used = if scene {
        saved.as_ref().is_some_and(|s| s.rooms.contains(room))
    } else {
        existing.is_some()
    };
    let detail = if scene {
        format!(
            "Hue scene · {} · Appears in this room’s Scenes button",
            value["room_name"].as_str().unwrap_or("")
        )
    } else {
        existing
            .map(|r| format!("Already in {r}"))
            .unwrap_or_else(|| {
                if value["resource_kind"] == "room" {
                    "Hue room · Control all its lights together".into()
                } else if value["on"].is_null() {
                    "Unavailable".into()
                } else {
                    "Light · On/off and brightness".into()
                }
            })
    };
    let title = name.clone();
    let room = room.clone();
    let connection_id = connection.id.clone();
    view!{<div class="card discovered-device"><strong>{title}</strong><p class="dim">{detail}</p>
        <button class="primary" disabled=move ||app.busy.get()||used on:click=move |_|{
            if scene {
                if let Some(mut saved)=saved.clone() {
                    if !saved.rooms.contains(&room) {saved.rooms.push(room.clone());}
                    app.run(api::put(format!("/api/scenes/{}",saved.id),saved));
                } else {
                    app.run(api::post("/api/scenes",json!({"name":name,"rooms":[room],"hue":{"connection_id":connection_id,"scene_id":id.strip_prefix("scene:").unwrap_or("")}})));
                }
            } else { app.run(api::post(format!("/api/rooms/{room}/devices"),json!({"name":name,"kind":"light","integration":{"via":"connection","connection_id":connection_id,"resource_id":id}}))); }
        }>{if used {"Added"} else {"Add to this room"}}</button>
    </div>}.into_any()
}
fn manual(app: App, connection: Connection, room: Id) -> AnyView {
    let infrared = connection.provider == Provider::Ir;
    let television = connection.provider == Provider::WebOs;
    let receiver = matches!(connection.provider, Provider::Denon{..});
    let existing = assigned(app, &connection, "");
    let name = RwSignal::new(if infrared {
        String::new()
    } else {
        connection.name.clone()
    });
    let codeset = RwSignal::new(String::new());
    let kind = RwSignal::new("tv".to_string());
    view!{<form on:submit=move |e|{e.prevent_default();let title=name.get_untracked().trim().to_string();if title.is_empty(){return}let resource=if infrared{codeset.get_untracked().trim().to_string()}else{String::new()};app.run(api::post(format!("/api/rooms/{room}/devices"),json!({"name":title,"kind":if infrared{kind.get_untracked()}else if television{"tv".into()}else if receiver{"speaker".into()}else{"media-player".into()},"integration":{"via":"connection","connection_id":connection.id,"resource_id":resource}})));}>
        {super::connections::field("Device name",name,"Living room TV")}
        {infrared.then(||view!{<label class="field">"Device type"<select aria-label="Device type" prop:value=move ||kind.get() on:change=move |e|kind.set(event_target_value(&e))><option value="tv">"TV"</option><option value="speaker">"Speaker"</option><option value="media-player">"Media player"</option><option value="other">"Other"</option></select></label>{super::connections::field("Codeset name",codeset,"lg-tv")}<p class="dim">"Use an installed codeset name. Codeset discovery, learning and sending are not available yet."</p>})}
        {existing.as_ref().map(|r|view!{<p>{format!("This device is already in {r}")}</p>})}
        <button type="submit" class="primary" disabled=move ||app.busy.get()||(!infrared && existing.is_some())>"Add to this room"</button>
    </form>}.into_any()
}
pub fn controls(app: App, device: &Device) -> AnyView {
    let config = app.config.get_untracked().unwrap_or_default();
    let (prefix, id) = match config.resolve_integration(&device.integration) {
        Some(Integration::Denon{..})=>return match &device.integration {
            Integration::Connection{connection_id,..}=>super::connections::denon_controls(app,connection_id.to_string()),_=>().into_any(),
        },
        Some(Integration::WebOs) => {
            let id=match &device.integration {Integration::Connection{connection_id,..}=>Some(connection_id.clone()),_=>config.connections.iter().find(|c|c.provider==Provider::WebOs).map(|c|c.id.clone())};
            return id.map(|id|super::webos::controls(app,format!("/api/connections/{id}/webos"))).unwrap_or_else(||().into_any());
        },
        Some(Integration::Hue { light_id }) => ("hue", light_id),
        Some(Integration::HomeAssistant { entity_id })
            if device.kind == couch_model::DeviceKind::Light =>
        {
            ("ha", entity_id)
        }
        _ => return ().into_any(),
    };
    let (base,id)=if let Some((connection,resource))=id.split_once('/') {(format!("/api/connections/{connection}/{prefix}"),resource.to_string())}else{(format!("/api/{prefix}"),id)};
    let base=StoredValue::new(base);
    let value = RwSignal::new(None::<Value>);
    let message = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    view!{<button class="ghost" disabled=move ||busy.get() on:click=move |_|{let id=id.clone();busy.set(true);spawn_local(async move{match api::ha("GET",&format!("{}/lights/{id}",base.get_value()),None).await{Ok(v)=>value.set(Some(v)),Err(e)=>{if e.unauthorized{app.paired.set(Some(false));}message.set(e.message);}}busy.set(false);});}>"Show light controls"</button><p role="status">{move ||message.get()}</p>{move ||value.get().map(|v|if prefix=="hue"{super::hue::controls(app,v,base.get_value())}else{super::home_assistant::controls(app,v,base.get_value())})}}.into_any()
}
