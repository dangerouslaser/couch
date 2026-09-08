//! Scenes: a name, and the list of things it does.
//!
//! A scene belongs to the house rather than to an area - the same "Movie
//! night" appears on the whole-home page and the downstairs one - so it is
//! edited here and attached from an area's screen.

use couch_model::{Action, Config, Icon, Id, Scene};
use leptos::prelude::*;
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
