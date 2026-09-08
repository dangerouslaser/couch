//! Scenes: a name, and the list of things it does.
//!
//! A scene belongs to the house rather than to an area - the same "Movie
//! night" appears on the whole-home page and the downstairs one - so it is
//! edited here and attached from an area's screen.

use couch_model::{Action, Config, Icon, Id, Scene};
use leptos::{prelude::*, task::spawn_local};
use serde_json::json;

use crate::route::Route;
use crate::screens::{counts, device_name, device_select, gone};
use crate::{api, ui, App};

pub fn list(app: App, config: &Config) -> AnyView {
    let rows = config
        .scenes
        .iter()
        .map(|scene| {
            let id = scene.id.clone();
            let open = id.clone();
            let name = scene.name.clone();
            let areas = config
                .areas
                .iter()
                .filter(|area| area.scenes.contains(&scene.id))
                .count();
            let subtitle = counts(&[
                (scene.steps.len(), "step", "steps"),
                (areas, "area", "areas"),
            ]);
            view! {
                <li class="row">
                    <button class="row-main" on:click=move |_| app.go(Route::Scene(open.clone()))>
                        <span class="row-title">{name}</span>
                        <span class="row-sub">{subtitle}</span>
                    </button>
                    {ui::danger_button("Delete", move || {
                        app.run(api::delete(format!("/api/scenes/{id}")))
                    })}
                </li>
            }
        })
        .collect_view();

    let empty = config.scenes.is_empty();
    view! {
        {ui::page_header(app, "Scenes".to_string(), None)}
        <p class="dim pad-x">
            "One press that puts several devices into a known state. Attach a scene \
             to an area to make it appear on that page."
        </p>
        {hue_import(app,config)}
        <ul class="rows">{rows}</ul>
        {empty.then(|| ui::empty("No scenes yet."))}
        {ui::add_row("New scene name", "Create scene", move |name| {
            app.run(api::post("/api/scenes", json!({ "name": name })))
        })}
    }
    .into_any()
}

pub fn detail(app: App, config: &Config, id: &Id) -> AnyView {
    let Some(scene) = config.scene(id) else {
        return gone(app, "That scene has been deleted.");
    };
    let id = id.clone();

    let save = {
        let id = id.clone();
        move |next: Scene| app.run(api::put(format!("/api/scenes/{id}"), next))
    };
    let base = scene.clone();
    let (for_name, for_icon, for_steps, for_add) =
        (save.clone(), save.clone(), save.clone(), save.clone());
    let (name_base, icon_base, steps_base, add_base) =
        (base.clone(), base.clone(), base.clone(), base.clone());
    let delete_id = id.clone();

    view! {
        {ui::page_header(app, scene.name.clone(), Some(Route::Scenes))}

        <section class="card">
            {ui::text_field("Name", scene.name.clone(), "Movie night", move |name| {
                for_name(Scene { name, ..name_base.clone() })
            })}
            {ui::icon_select(scene.icon, move |icon: Option<Icon>| {
                for_icon(Scene { icon, ..icon_base.clone() })
            })}
        </section>

        {room_assignment(app,config,scene)}
        {scene.hue.as_ref().map(|_|view!{<section class="card"><h2>"Hue scene"</h2><p>"This recalls the scene saved on your bridge. Edit its lighting in the Hue app."</p></section>})}
        <div hidden=scene.hue.is_some()>
        <h2 class="section">"Device commands"</h2>
        <p class="dim">"Add commands in the order they should run. For example, turn on the TV, then select its input. Command names depend on the integration; saving does not test or send them."</p>
        {scene.steps.is_empty().then(|| {
            ui::empty("This scene does nothing yet. Add a step below.")
        })}
        <ul class="rows">
            {scene.steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let commit = for_steps.clone();
                    let base = steps_base.clone();
                    step_row(config, step, move |next| {
                        let mut scene = base.clone();
                        match next {
                            Some(action) => scene.steps[index] = action,
                            None => {
                                scene.steps.remove(index);
                            }
                        }
                        commit(scene);
                    })
                })
                .collect_view()}
        </ul>

        {add_step(config, move |action| {
            let mut scene = add_base.clone();
            scene.steps.push(action);
            for_add(scene);
        })}

        </div>
        <div class="pad"><button class="ghost" on:click=move |_| app.go(Route::Areas)>"Add this scene to a remote screen →"</button></div>
        <div class="pad">
            {ui::danger_button("Delete this scene", move || {
                app.go(Route::Scenes);
                app.run(api::delete(format!("/api/scenes/{delete_id}")));
            })}
        </div>
    }
    .into_any()
}

/// One step. `commit(None)` removes it.
fn step_row(
    config: &Config,
    step: &Action,
    commit: impl Fn(Option<Action>) + Clone + 'static,
) -> AnyView {
    let label = device_name(config, &step.device);
    let (for_command, for_delete) = (commit.clone(), commit);
    let base = step.clone();

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
                    if !command.is_empty() && command != base.command {
                        for_command(Some(Action { command, ..base.clone() }));
                    }
                }
            />
            <button class="ghost" on:click=move |_| for_delete(None)>"Remove"</button>
        </li>
    }
    .into_any()
}

/// Picking a device is what adds the step; the command starts at "on" and is
/// edited in place, which is one interaction rather than a form.
fn add_step(config: &Config, commit: impl Fn(Action) + 'static) -> AnyView {
    if config.devices().next().is_none() {
        return ui::empty("Add a device to a room before building a scene.");
    }
    view! {
        <div class="add-row">
            <span class="label">"Add a step"</span>
            {device_select(config, None, true, false, move |picked| {
                if let Some(device) = picked {
                    commit(Action::new(device, "on"));
                }
            })}

        </div>
    }
    .into_any()
}

fn room_assignment(app: App, config: &Config, scene: &Scene) -> AnyView {
    let scene = scene.clone();
    view!{<section class="card"><h2>"Show in rooms"</h2><p>"Selected rooms get this scene in their bottom Scenes button. Home screen scene selection stays in Remote screens."</p>
        {config.rooms.iter().map(|r|{let id=r.id.clone();let name=r.name.clone();let base=scene.clone();let checked=scene.rooms.contains(&id);view!{<label class="room-assignment"><input type="checkbox" checked=checked on:change=move |e|{let mut next=base.clone();if event_target_checked(&e){if !next.rooms.contains(&id){next.rooms.push(id.clone());}}else{next.rooms.retain(|r|r!=&id);}app.run(api::put(format!("/api/scenes/{}",next.id),next));}/>{name}</label>}}).collect_view()}
    </section>}.into_any()
}
fn hue_import(app: App, config: &Config) -> AnyView {
    let Some(connection) = config
        .connections
        .iter()
        .find(|c| c.provider == couch_model::Provider::Hue)
    else {
        return ().into_any();
    };
    let connection = connection.id.clone();
    let rooms = config.rooms.clone();
    let room = RwSignal::new(String::new());
    let search = RwSignal::new(String::new());
    let bridge_room = RwSignal::new(String::new());
    let list = RwSignal::new(Vec::<serde_json::Value>::new());
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    view!{<section class="card"><h2>"Import Hue scenes"</h2><p>"Choose a scene saved on your bridge. It appears under Scenes on the remote, with optional room assignment."</p>
        <label class="field">"Show in room"<select aria-label="Show in room" prop:value=move ||room.get() on:change=move |e|room.set(event_target_value(&e))><option value="">"Home only — assign rooms later"</option>{rooms.into_iter().map(|r|view!{<option value=r.id.to_string()>{r.name}</option>}).collect_view()}</select></label>
        <button class="ghost" disabled=move ||busy.get() on:click=move |_|{busy.set(true);spawn_local(async move {match api::ha("GET","/api/hue/scenes",None).await{Ok(v)=>{list.set(v.as_array().cloned().unwrap_or_default());message.set(String::new());},Err(e)=>{if e.unauthorized{app.paired.set(Some(false));}message.set(e.message);}}busy.set(false);});}>"Find Hue scenes"</button>
        <label class="field">"Search Hue scenes"<input type="search" placeholder="Scene or bridge room name" prop:value=move ||search.get() on:input=move |e|search.set(event_target_value(&e))/></label>
        <label class="field">"Hue room or zone"<select aria-label="Hue room or zone" prop:value=move ||bridge_room.get() on:change=move |e|bridge_room.set(event_target_value(&e))><option value="">"All bridge rooms and zones"</option>{move ||list.get().iter().filter_map(|v|v["room_name"].as_str()).filter(|s|!s.is_empty()).map(str::to_string).collect::<std::collections::BTreeSet<_>>().into_iter().map(|name|view!{<option value=name.clone()>{name.clone()}</option>}).collect_view()}</select></label>
        <p role="status">{move ||message.get()}</p>
        <div class="discovered-devices">{move ||list.get().into_iter().filter(|v| {let text=format!("{} {}",v["name"].as_str().unwrap_or(""),v["room_name"].as_str().unwrap_or("")).to_lowercase(); search.get().to_lowercase().split_whitespace().all(|word|text.contains(word)) && (bridge_room.get().is_empty() || v["room_name"].as_str()==Some(bridge_room.get().as_str()))}).map(|v|{let name=v["name"].as_str().unwrap_or("Hue scene").to_string();let label=name.clone();let id=v["entity_id"].as_str().unwrap_or("").trim_start_matches("scene:").to_string();let connection=connection.clone();let used=app.config.get().is_some_and(|c|c.scenes.iter().any(|s|s.hue.as_ref().is_some_and(|h|h.connection_id==connection && h.scene_id==id)));let bridge_room=v["room_name"].as_str().unwrap_or("").to_string();view!{<div class="card hue-scene"><strong>{label}</strong><p class="dim">{bridge_room}</p><button class="primary" disabled=move ||app.busy.get()||used on:click=move |_|{let rooms=if room.get_untracked().is_empty(){Vec::<String>::new()}else{vec![room.get_untracked()]};app.run(api::post("/api/scenes",json!({"name":name,"rooms":rooms,"hue":{"connection_id":connection,"scene_id":id}})));}>{if used{"Imported"}else{"Import scene"}}</button></div>}}).collect_view()}</div>
    </section>}.into_any()
}
