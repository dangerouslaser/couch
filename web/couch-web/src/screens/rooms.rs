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
                    "This room is not in any area, so the remote will not show it.".to_string()
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
        {device_form(app, &add_id, None)}

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
    view! { <li class="card device"><details>
        <summary><strong>{device.name.clone()}</strong><span class="row-sub">{super::overview::connection_summary(&device.integration)}</span><span class="destination-action">"Edit device"</span></summary>
        {device_form(app, room, Some(device))}
    </details></li> }.into_any()
}

/// One explicit save keeps connection changes atomic and preserves the old
/// connection until the user has supplied a complete replacement.
fn device_form(app: App, room: &Id, device: Option<&Device>) -> AnyView {
    let existing = device.cloned();
    let name = RwSignal::new(device.map(|d| d.name.clone()).unwrap_or_default());
    let kind = RwSignal::new(device.map(|d| d.kind).unwrap_or_default());
    let via = RwSignal::new(
        device
            .map(|d| d.integration.via().to_string())
            .unwrap_or("none".into()),
    );
    let (initial_host, initial_port, initial_entity, initial_codeset) =
        match device.map(|d| &d.integration) {
            Some(Integration::Kodi { host, port }) => {
                (host.clone(), port.to_string(), String::new(), String::new())
            }
            Some(Integration::HomeAssistant { entity_id }) => (
                String::new(),
                "9090".into(),
                entity_id.clone(),
                String::new(),
            ),
            Some(Integration::Ir { codeset }) => {
                (String::new(), "9090".into(), String::new(), codeset.clone())
            }
            _ => (String::new(), "9090".into(), String::new(), String::new()),
        };
    let host = RwSignal::new(initial_host);
    let port = RwSignal::new(initial_port);
    let entity = RwSignal::new(initial_entity);
    let codeset = RwSignal::new(initial_codeset);
    let initial = (
        name.get_untracked(),
        kind.get_untracked(),
        via.get_untracked(),
        host.get_untracked(),
        port.get_untracked(),
        entity.get_untracked(),
        codeset.get_untracked(),
    );
    let error = RwSignal::new(Option::<String>::None);
    let room_id = room.clone();
    let delete_room = room.clone();
    let delete_device = device.map(|d| d.id.clone());
    let editing = device.is_some();
    let original_via = via.get_untracked();
    view! { <form class="device-form" on:submit=move |event| {
        event.prevent_default();
        if app.busy.get_untracked() { return; }
        let integration = match validated_connection(&via.get(), &host.get(), &port.get(), &entity.get(), &codeset.get()) {
            Ok(value) => value,
            Err(message) => { error.set(Some(message)); return; }
        };
        let name = name.get().trim().to_string();
        if name.is_empty() { error.set(Some("Enter a device name.".into())); return; }
        error.set(None);
        match &existing {
            Some(base) => app.run(api::put(format!("/api/rooms/{room_id}/devices/{}", base.id), Device {name, kind:kind.get(), integration, ..base.clone()})),
            None => app.run(api::post(format!("/api/rooms/{room_id}/devices"), json!({"name":name,"kind":kind.get(),"integration":integration}))),
        }
    }>
        <h3>{if editing { "Device settings" } else { "Add a device" }}</h3>
        <p class="dim">"Name the device, choose what it is, then choose how to connect. Save when all fields are ready."</p>
        {draft_field("Device name", name, "Living room TV")}
        <label class="field"><span class="label">"Device type"</span><select aria-label="Device type" prop:value=move || kind.get().name() on:change=move |e| kind.set(DeviceKind::from_name(&event_target_value(&e)).unwrap_or_default())>
        {ALL_DEVICE_KINDS.iter().map(|k| view! { <option value=k.name()>{k.name().replace('-', " ")}</option> }).collect_view()}</select></label>
        <label class="field"><span class="label">"Connection / client"</span><select aria-label="Connection / client" prop:value=move || via.get() on:change=move |e| via.set(event_target_value(&e))>
        <option value="none">"Set up later"</option><option value="kodi">"Kodi"</option><option value="home-assistant">"Home Assistant"</option><option value="ir">"Infrared (IR)"</option></select></label>
        <Show when=move || via.get() == "kodi">{draft_field("Hostname or IP address", host, "192.168.1.20")}{draft_field("TCP port", port, "9090")}</Show>
        <Show when=move || via.get() == "home-assistant">{draft_field("Entity ID", entity, "light.living_room")}</Show>
        <Show when=move || via.get() == "ir">{draft_field("Codeset name", codeset, "lg-tv")}</Show>
        <Show when=move || editing && via.get() != original_via><p class="notice">"Saving replaces this device’s previous connection settings. Switching back before saving keeps them."</p></Show>
        {move || error.get().map(|message| view! { <p class="wrong" role="alert">{message}</p> })}
        <button type="submit" class="primary" disabled=move || app.busy.get()>{if editing { "Save device" } else { "Add device" }}</button>
        <button type="button" class="ghost" on:click=move |_| {
            name.set(initial.0.clone()); kind.set(initial.1); via.set(initial.2.clone());
            host.set(initial.3.clone()); port.set(initial.4.clone()); entity.set(initial.5.clone()); codeset.set(initial.6.clone()); error.set(None);
        }>{if editing { "Discard changes" } else { "Clear form" }}</button>
        {delete_device.map(|id| view! { <div class="card-foot">{ui::danger_button("Delete device", move || app.run(api::delete(format!("/api/rooms/{delete_room}/devices/{id}"))))}<span class="dim">"Also removes its commands from scenes and activities."</span></div> })}
    </form> }.into_any()
}

fn draft_field(label: &'static str, value: RwSignal<String>, placeholder: &'static str) -> AnyView {
    view! { <label class="field"><span class="label">{label}</span><input type="text" placeholder=placeholder prop:value=move || value.get() on:input=move |e| value.set(event_target_value(&e))/></label> }.into_any()
}

fn validated_connection(
    via: &str,
    host: &str,
    port: &str,
    entity: &str,
    codeset: &str,
) -> Result<Integration, String> {
    match via {
        "none" => Ok(Integration::None),
        "kodi" => {
            let port = port
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or("Enter a TCP port from 1 to 65535.")?;
            if host.trim().is_empty() {
                return Err("Enter the Kodi hostname or IP address.".into());
            }
            Ok(Integration::Kodi {
                host: host.trim().into(),
                port,
            })
        }
        "home-assistant" if !entity.trim().is_empty() => Ok(Integration::HomeAssistant {
            entity_id: entity.trim().into(),
        }),
        "ir" if !codeset.trim().is_empty() => Ok(Integration::Ir {
            codeset: codeset.trim().into(),
        }),
        "home-assistant" => Err("Enter the Home Assistant entity ID.".into()),
        "ir" => Err("Enter an infrared codeset name, or choose Set up later.".into()),
        _ => Err("Choose a connection type.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_form_rejects_incomplete_and_invalid_settings() {
        for port in ["", "0", "65536", "abc"] {
            assert!(validated_connection("kodi", "kodi.local", port, "", "").is_err());
        }
        assert!(validated_connection("kodi", " ", "9090", "", "").is_err());
        assert!(validated_connection("ir", "", "", "", " ").is_err());
        assert!(validated_connection("home-assistant", "", "", " ", "").is_err());
        assert_eq!(
            validated_connection("kodi", " kodi.local ", "9090", "", "").unwrap(),
            Integration::Kodi {
                host: "kodi.local".into(),
                port: 9090
            }
        );
        assert_eq!(
            validated_connection("none", "", "", "", "").unwrap(),
            Integration::None
        );
    }
}
