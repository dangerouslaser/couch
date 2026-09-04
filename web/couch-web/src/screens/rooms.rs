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
        {ui::page_header(app, room.name.clone(), Some(Route::Areas))}

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
        {ui::add_row("New device name", "Add", move |name| {
            app.run(api::post(
                format!("/api/rooms/{add_id}/devices"),
                json!({ "name": name }),
            ))
        })}

        <div class="pad">
            {ui::danger_button("Delete this room", move || {
                app.go(Route::Areas);
                app.run(api::delete(format!("/api/rooms/{delete_id}")));
            })}
            <p class="dim">
                "Deleting a room removes it from every area, and removes its devices \
                 from every scene."
            </p>
        </div>
    }
    .into_any()
}

/// One device, editable in place.
///
/// Every control writes the whole device back, because the endpoint replaces
/// it: sending only the changed field would need a merge on the server and a
/// second shape for it to merge into.
fn device_card(app: App, room: &Id, device: &Device) -> AnyView {
    let save = {
        let (room, id) = (room.clone(), device.id.clone());
        move |next: Device| {
            app.run(api::put(format!("/api/rooms/{room}/devices/{id}"), next));
        }
    };
    let base = device.clone();
    let (for_name, for_kind, for_integration) = (save.clone(), save.clone(), save);
    let (name_base, kind_base, integration_base) = (base.clone(), base.clone(), base.clone());
    let selected_kind = device.kind.name();
    let delete = {
        let (room, id) = (room.clone(), device.id.clone());
        move || app.run(api::delete(format!("/api/rooms/{room}/devices/{id}")))
    };

    view! {
        <li class="card device">
            {ui::text_field("Name", device.name.clone(), "LG C3", move |name| {
                for_name(Device { name, ..name_base.clone() })
            })}

            <label class="field">
                <span class="label">"Kind"</span>
                <select on:change=move |ev| {
                    let kind = DeviceKind::from_name(&event_target_value(&ev)).unwrap_or_default();
                    for_kind(Device { kind, ..kind_base.clone() });
                }>
                    {ALL_DEVICE_KINDS
                        .iter()
                        .map(|kind| {
                            let name = kind.name();
                            view! {
                                <option value=name selected=name == selected_kind>{name}</option>
                            }
                        })
                        .collect_view()}
                </select>
            </label>

            {integration_editor(base.clone(), move |integration| {
                for_integration(Device { integration, ..integration_base.clone() })
            })}

            <div class="card-foot">
                <span class="dim mono">{device.id.to_string()}</span>
                {ui::danger_button("Remove", delete)}
            </div>
        </li>
    }
    .into_any()
}

/// How this device is reached: the transport, and the one or two fields it
/// needs.
///
/// Switching transport resets the fields rather than trying to carry a host
/// across to an entity id. They are not the same thing, and a half-migrated
/// address that looks plausible is worse than a blank one.
fn integration_editor(device: Device, commit: impl Fn(Integration) + Clone + 'static) -> AnyView {
    let via = device.integration.via();
    let on_via = commit.clone();
    let fields = match device.integration.clone() {
        Integration::None => ().into_any(),
        Integration::Kodi { host, port } => {
            let (host_commit, port_commit) = (commit.clone(), commit.clone());
            let (host_for_port, port_for_host) = (host.clone(), port);
            view! {
                {ui::text_field("Host", host, "kodi.local", move |host| {
                    host_commit(Integration::Kodi { host, port: port_for_host })
                })}
                {ui::text_field("Port", port.to_string(), "9090", move |text| {
                    if let Ok(port) = text.parse::<u16>() {
                        port_commit(Integration::Kodi { host: host_for_port.clone(), port });
                    }
                })}
            }
            .into_any()
        }
        Integration::HomeAssistant { entity_id } => ui::text_field(
            "Entity id",
            entity_id,
            "light.living_room",
            move |entity_id| commit(Integration::HomeAssistant { entity_id }),
        ),
        Integration::Ir { codeset } => {
            ui::text_field("Codeset", codeset, "lg-tv", move |codeset| {
                commit(Integration::Ir { codeset })
            })
        }
    };

    view! {
        <label class="field">
            <span class="label">"Reached via"</span>
            <select on:change=move |ev| {
                on_via(match event_target_value(&ev).as_str() {
                    "kodi" => Integration::Kodi { host: String::new(), port: 9090 },
                    "home-assistant" => Integration::HomeAssistant { entity_id: String::new() },
                    "ir" => Integration::Ir { codeset: String::new() },
                    _ => Integration::None,
                });
            }>
                {["none", "kodi", "home-assistant", "ir"]
                    .into_iter()
                    .map(|option| view! {
                        <option value=option selected=option == via>{option}</option>
                    })
                    .collect_view()}
            </select>
        </label>
        {fields}
    }
    .into_any()
}
