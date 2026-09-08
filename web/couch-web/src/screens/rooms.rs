//! One room: what it is called, and what is in it.
//!
//! The device list is the interesting part, because it is what the remote's
//! room row is rendered from. The screen shows that rendering back - "5 devices
//! · Kodi, Hue, LG C3" - so the effect of adding a device is visible where it
//! will actually appear, rather than only in a list.

use couch_model::{Config, Device, DeviceKind, Icon, Id, Integration, ALL_DEVICE_KINDS};
use leptos::prelude::*;
use serde_json::json;

use crate::route::Route;
use crate::screens::{counts, gone};
use crate::{api, ui, App};

pub fn detail(app: App, config: &Config, id: &Id) -> AnyView {
    let Some(room) = config.room(id) else {
        return gone(app, "That room has been deleted.");
    };
    let id = id.clone();

    let save = {
        let id = id.clone();
        move |name: String, icon: Option<Icon>| {
            app.run(api::put(
                format!("/api/rooms/{id}"),
                json!({ "name": name, "icon": icon }),
            ));
        }
    };
    let (for_name, for_icon) = (save.clone(), save);
    let current_icon = room.icon;
    let current_name = room.name.clone();

    // Exactly what the remote's row will say, from the model's own helpers.
    let rendered = match room.device_detail().as_str() {
        "" => room.device_summary(),
        detail => format!("{} · {detail}", room.device_summary()),
    };

    let in_areas: Vec<String> = config
        .areas
        .iter()
        .filter(|area| area.rooms.contains(&room.id))
        .map(|area| area.name.clone())
        .collect();

    let add_id = id.clone();
    let delete_id = id.clone();

    view! {
        {ui::page_header(app, room.name.clone(), Some(Route::Rooms))}

        <section class="card">
            {ui::text_field("Name", room.name.clone(), "Living room", move |name| {
                for_name(name, current_icon)
            })}
            {ui::icon_select(room.icon, move |icon| for_icon(current_name.clone(), icon))}
            <p class="preview">
                <span class="label">"On the remote"</span>
                <span>{rendered}</span>
            </p>
            <p class="dim">
                {if in_areas.is_empty() {
                    "Available in All rooms on the remote. Add it to a custom screen when ready.".to_string()
                } else {
                    format!("Shown in: {}", in_areas.join(", "))
                }}
            </p>
        </section>

        <h2 class="section">
            "Devices" <span class="count">{counts(&[(room.devices.len(), "device", "devices")])}</span>
        </h2>
        {room.devices.is_empty().then(|| ui::empty("No devices here yet."))}
        <ul class="rows">
            {room.devices
                .iter()
                .map(|device| device_card(app, &id, device))
                .collect_view()}
        </ul>
        {super::device_picker::picker(app, config, &add_id)}

        <div class="pad">
            {ui::danger_button("Delete this room", move || {
                app.go(Route::Rooms);
                app.run(api::delete(format!("/api/rooms/{delete_id}")));
            })}
            <p class="dim">
                "Deleting this room also deletes its devices and activities, removes it from all screens, and removes commands for its devices from scenes."
            </p>
        </div>
    }
    .into_any()
}

fn device_card(app: App, room: &Id, device: &Device) -> AnyView {
    let name = RwSignal::new(device.name.clone());
    let kind = RwSignal::new(device.kind);
    let original_name = device.name.clone();
    let original_kind = device.kind;
    let base = device.clone();
    let room = room.clone();
    let delete_room = room.clone();
    let delete_id = device.id.clone();
    let summary = app
        .config
        .get_untracked()
        .and_then(|c| match &device.integration {
            Integration::Connection { connection_id, .. } => {
                c.connection(connection_id).map(super::connections::label)
            }
            _ => None,
        })
        .unwrap_or_else(|| super::overview::connection_summary(&device.integration));
    view!{<li class="card device"><h3>{device.name.clone()}</h3><p class="dim">{summary}</p>
        {super::device_picker::controls(app,device)}
        <details><summary>"Edit device"</summary><form on:submit=move |e|{e.prevent_default();let title=name.get_untracked().trim().to_string();if title.is_empty(){return}app.run(api::put(format!("/api/rooms/{room}/devices/{}",base.id),Device{name:title,kind:kind.get_untracked(),..base.clone()}));}>
        {super::connections::field("Device name",name,"Device name")}
        <label class="field">"Device type"<select aria-label="Device type" prop:value=move ||kind.get().name() on:change=move |e|kind.set(DeviceKind::from_name(&event_target_value(&e)).unwrap_or_default())>{ALL_DEVICE_KINDS.iter().map(|k|view!{<option value=k.name()>{k.name()}</option>}).collect_view()}</select></label>
        <p class="dim">"Manage server and bridge settings in Connections. To use another source device, remove this device and add the replacement from its connection."</p>
        <button class="primary" type="submit">"Save device"</button><button class="ghost" type="button" on:click=move |_|{name.set(original_name.clone());kind.set(original_kind);}>"Discard changes"</button></form></details>
        {ui::danger_button("Delete device",move ||app.run(api::delete(format!("/api/rooms/{delete_room}/devices/{delete_id}"))))}
    </li>}.into_any()
}
