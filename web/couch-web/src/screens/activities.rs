//! Activities: "Watch TV", not "the TV is on".
//!
//! An activity names a thing you sit down to do, in one room, driven by one
//! source device. The remote shows the ones that are running in its activity
//! strip; which areas draw it is set from an area's own screen, because "what
//! is worth a strip on the whole-home page" is a judgement rather than a
//! consequence of where the room is.

use couch_model::{Action, Activity, ActivityKind, Config, Id};
use leptos::prelude::*;
use serde_json::json;

use crate::route::Route;
use crate::screens::{counts, device_name, device_select, gone};
use crate::{api, ui, App};

pub fn list(app: App, config: &Config) -> AnyView {
    let rows = config
        .activities
        .iter()
        .map(|activity| {
            let id = activity.id.clone();
            let open = id.clone();
            let name = activity.name.clone();
            let room = config
                .room(&activity.room)
                .map(|r| r.name.clone())
                .unwrap_or_else(|| format!("{} (missing)", activity.room));
            let kind = match activity.kind {
                ActivityKind::Audio => "audio",
                ActivityKind::Video => "video",
            };
            let areas = config
                .areas
                .iter()
                .filter(|area| area.activities.contains(&activity.id))
                .count();
            let subtitle = format!(
                "{room} · {kind} · {}",
                counts(&[
                    (activity.steps.len(), "step", "steps"),
                    (areas, "area", "areas"),
                ])
            );

            view! {
                <li class="row">
                    <button
                        class="row-main"
                        on:click=move |_| app.go(Route::Activity(open.clone()))
                    >
                        <span class="row-title">{name}</span>
                        <span class="row-sub">{subtitle}</span>
                    </button>
                    {ui::danger_button("Delete", move || {
                        app.run(api::delete(format!("/api/activities/{id}")))
                    })}
                </li>
            }
        })
        .collect_view();

    view! {
        {ui::page_header(app, "Activities".to_string(), None)}
        <p class="dim pad-x">
            "An activity is something you do in a room, such as Watch TV or Listen to music. Choose its source device and startup commands, then add it to a remote screen."
        </p>

        <ul class="rows">{rows}</ul>
        {config.activities.is_empty().then(|| ui::empty("No activities yet."))}
        {new_activity(app, config)}
    }
    .into_any()
}

/// Creating one needs a room as well as a name, so this is a two-field form
/// rather than the shared single-field add row.
///
/// The room is read out of the `<select>` when Add is pressed rather than
/// mirrored into a signal as the name is. Adding an activity replaces the
/// config, which rebuilds this form with a fresh signal at the first room -
/// while the select, being diffed rather than recreated, still shows what was
/// picked. The element is the only copy that cannot drift from itself.
fn new_activity(app: App, config: &Config) -> AnyView {
    if config.rooms.is_empty() {
        return ui::empty("Create a room first - an activity has to happen somewhere.");
    }
    let draft = RwSignal::new(String::new());
    let picked: NodeRef<leptos::html::Select> = NodeRef::new();
    let rooms: Vec<(String, String)> = config
        .rooms
        .iter()
        .map(|r| (r.id.to_string(), r.name.clone()))
        .collect();

    view! {
        <div class="add-row">
            <input
                type="text"
                aria-label="Activity name"
                placeholder="New activity name"
                prop:value=move || draft.get()
                on:input=move |ev| draft.set(event_target_value(&ev))
            />
            <select aria-label="Activity room" node_ref=picked>
                {rooms
                    .into_iter()
                    .map(|(id, name)| view! { <option value=id>{name}</option> })
                    .collect_view()}
            </select>
            <button
                class="primary"
                on:click=move |_| {
                    let name = draft.get().trim().to_string();
                    let room = picked.get_untracked().map(|el| el.value()).unwrap_or_default();
                    if !name.is_empty() && !room.is_empty() {
                        draft.set(String::new());
                        app.run(api::post(
                            "/api/activities",
                            json!({ "name": name, "room": room }),
                        ));
                    }
                }
            >"Create activity"</button>
        </div>
    }
    .into_any()
}

pub fn detail(app: App, config: &Config, id: &Id) -> AnyView {
    let Some(activity) = config.activity(id) else {
        return gone(app, "That activity has been deleted.");
    };
    let id = id.clone();

    // One writer for the whole screen. It captures only `app` (a Copy handle)
    // and the id, so it is `Clone` and each control takes its own copy along
    // with the activity it is editing.
    let save = {
        let id = id.clone();
        move |next: Activity| app.run(api::put(format!("/api/activities/{id}"), next))
    };
    let base = activity.clone();

    let name_save = save.clone();
    let name_base = base.clone();
    let kind_save = save.clone();
    let kind_base = base.clone();
    let room_save = save.clone();
    let room_base = base.clone();
    let source_save = save.clone();
    let source_base = base.clone();
    let add_save = save.clone();
    let add_base = base.clone();

    let rooms: Vec<(String, String)> = config
        .rooms
        .iter()
        .map(|r| (r.id.to_string(), r.name.clone()))
        .collect();
    let current_room = activity.room.to_string();
    let current_kind = activity.kind;
    let delete_id = id.clone();
    let has_devices = config.devices().next().is_some();
    // Which pages of the remote will draw this while it runs. An activity with
    // no area is configured but invisible, which is worth saying out loud.
    let strips: Vec<String> = config
        .areas
        .iter()
        .filter(|area| area.activities.contains(&activity.id))
        .map(|area| area.name.clone())
        .collect();
    let shown_in = if strips.is_empty() {
        "On no area's strip yet, so the remote will not show it running.".to_string()
    } else {
        format!("On the strip of: {}", strips.join(", "))
    };

    view! {
        {ui::page_header(app, activity.name.clone(), Some(Route::Activities))}

        <section class="card">
            {ui::text_field("Name", activity.name.clone(), "Watch TV", move |name| {
                name_save(Activity { name, ..name_base.clone() })
            })}

            <label class="field">
                <span class="label">"Activity type"</span>
                <select on:change=move |ev| {
                    let kind = if event_target_value(&ev) == "video" {
                        ActivityKind::Video
                    } else {
                        ActivityKind::Audio
                    };
                    kind_save(Activity { kind, ..kind_base.clone() });
                }>
                    <option value="audio" selected=current_kind == ActivityKind::Audio>
                        "audio (bars)"
                    </option>
                    <option value="video" selected=current_kind == ActivityKind::Video>
                        "video (play triangle)"
                    </option>
                </select>
            </label>

            <label class="field">
                <span class="label">"Room"</span>
                <select on:change=move |ev| {
                    let room = Id::new(event_target_value(&ev));
                    room_save(Activity { room, ..room_base.clone() });
                }>
                    {rooms
                        .into_iter()
                        .map(|(id, name)| {
                            let is = id == current_room;
                            view! { <option value=id selected=is>{name}</option> }
                        })
                        .collect_view()}
                </select>
            </label>

            <label class="field">
                <span class="label">"Source device"</span>
                {device_select(config, activity.source.as_ref(), true, true, move |source| {
                    source_save(Activity { source, ..source_base.clone() });
                })}

            </label>
            <p class="dim">"The device the transport keys drive while this is running."</p>
            <p class="dim">{shown_in}</p>
        </section>


        <h2 class="section">"Startup commands"</h2>
        <p class="dim">"Commands are listed in execution order. Pick a device to add an on command, then edit the command for that device. Command names depend on its integration; saving does not test or send them."</p>
        {activity.steps.is_empty().then(|| {
            ui::empty("Nothing is brought up when this starts.")
        })}
        <ul class="rows">
            {activity.steps
                .iter()
                .enumerate()
                .map(|(index, step)| step_row(config, step, index, save.clone(), base.clone()))
                .collect_view()}
        </ul>

        {has_devices.then(|| view! {
            <div class="add-row">
                <span class="label">"Add a step"</span>
                {device_select(config, None, true, false, move |picked| {
                    if let Some(device) = picked {
                        let mut next = add_base.clone();
                        next.steps.push(Action::new(device, "on"));
                        add_save(next);
                    }
                })}

            </div>
        })}

        <div class="pad"><button class="ghost" on:click=move |_| app.go(Route::Areas)>"Choose a remote screen for this activity →"</button></div>
        <div class="pad">
            {ui::danger_button("Delete this activity", move || {
                app.go(Route::Activities);
                app.run(api::delete(format!("/api/activities/{delete_id}")));
            })}
        </div>
    }
    .into_any()
}

fn step_row(
    config: &Config,
    step: &Action,
    index: usize,
    save: impl Fn(Activity) + Clone + 'static,
    base: Activity,
) -> AnyView {
    let label = device_name(config, &step.device);
    let step_base = step.clone();
    let delete_save = save.clone();
    let delete_base = base.clone();

    view! {
        <li class="row step">
            <span class="row-title">{label}</span>
            <input
                class="command"
                aria-label="Device command"
                type="text"
                value=step.command.clone()
                placeholder="on"
                on:change=move |ev| {
                    let command = event_target_value(&ev).trim().to_string();
                    if !command.is_empty() && command != step_base.command {
                        let mut next = base.clone();
                        next.steps[index] = Action { command, ..step_base.clone() };
                        save(next);
                    }
                }
            />
            <button
                class="ghost"
                on:click=move |_| {
                    let mut next = delete_base.clone();
                    next.steps.remove(index);
                    delete_save(next);
                }
            >"Remove"</button>
        </li>
    }
    .into_any()
}
