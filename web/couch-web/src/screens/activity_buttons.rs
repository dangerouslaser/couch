//! Activity-scoped physical mappings; layout follows the HA100 front-panel photo.
use crate::{api, App};
use couch_model::{
    buttons::{functions, Binding, Button, Gesture},
    Action, Activity, Config, Id,
};
use leptos::prelude::*;

// Percent positions within the physical panel. HTML buttons preserve keyboard
// focus, names and hit targets instead of making the editor a flat bitmap.
const KEYS: &[(Button, &str, &str, i32, i32)] = &[
    (Button::Back, "Back", "↶", 18, 40),
    (Button::Home, "Home", "⌂", 50, 40),
    (Button::Power, "Power", "⏻", 82, 40),
    (Button::Up, "D-pad up", "▲", 50, 49),
    (Button::Down, "D-pad down", "▼", 50, 66),
    (Button::Left, "D-pad left", "◀", 29, 57),
    (Button::Right, "D-pad right", "▶", 71, 57),
    (Button::Ok, "OK", "OK", 50, 57),
    (Button::VolumeUp, "Volume up", "V+", 12, 49),
    (Button::VolumeDown, "Volume down", "V−", 12, 65),
    (Button::ChannelUp, "Channel up", "C+", 88, 49),
    (Button::ChannelDown, "Channel down", "C−", 88, 65),
    (Button::Mute, "Mute", "Mute", 18, 74),
    (Button::Microphone, "Microphone", "Mic", 50, 74),
    (Button::Menu, "Menu", "☰", 82, 74),
    (Button::Lights, "Lights shortcut", "☼", 17, 83),
    (Button::Activity, "Activity shortcut", "▥", 39, 83),
    (Button::Music, "Music shortcut", "♫", 61, 83),
    (Button::Tv, "TV shortcut", "TV", 83, 83),
    (Button::Red, "Red", "R", 17, 91),
    (Button::Green, "Green", "G", 39, 91),
    (Button::Blue, "Blue", "B", 61, 91),
    (Button::Yellow, "Yellow", "Y", 83, 91),
];
pub fn editor(app: App, config: &Config, activity: &Activity) -> AnyView {
    let config = StoredValue::new(config.clone());
    let activity = StoredValue::new(activity.clone());
    let selected = RwSignal::new(app.button_selection.get_untracked().0);
    let gesture = RwSignal::new(app.button_selection.get_untracked().1);
    let device = RwSignal::new(String::new());
    let command = RwSignal::new(String::new());
    let mode = RwSignal::new("default".to_string());
    let load = move |button| {
        selected.set(button);
        if !button.supports_long() {
            gesture.set(Gesture::Short);
        }
        app.button_selection.set((button, gesture.get_untracked()));
        let binding = activity
            .get_value()
            .buttons
            .into_iter()
            .find(|b| b.button == button && b.gesture == gesture.get_untracked());
        match binding {
            None => {
                mode.set("default".into());
                device.set(String::new());
                command.set(String::new());
            }
            Some(Binding { action: None, .. }) => {
                mode.set("disabled".into());
                device.set(String::new());
                command.set(String::new());
            }
            Some(Binding {
                action: Some(action),
                ..
            }) => {
                mode.set("device".into());
                device.set(action.device.to_string());
                command.set(action.command);
            }
        }
    };
    load(selected.get_untracked());
    let mapping_label = move |button| {
        let activity = activity.get_value();
        match activity
            .buttons
            .iter()
            .find(|b| b.button == button && b.gesture == gesture.get())
        {
            None => "Activity default".to_string(),
            Some(Binding { action: None, .. }) => "Do nothing".to_string(),
            Some(Binding {
                action: Some(a), ..
            }) => {
                let config = config.get_value();
                let target = config.devices().find(|(_, d)| d.id == a.device);
                let function = target
                    .and_then(|(_, d)| config.resolve_integration(&d.integration))
                    .and_then(|i| functions(&i).iter().find(|f| f.0 == a.command).map(|f| f.1))
                    .unwrap_or(&a.command);
                format!(
                    "{} · {function}",
                    target
                        .map(|(_, d)| d.name.as_str())
                        .unwrap_or("Removed device")
                )
            }
        }
    };
    let options: Vec<_> = config
        .get_value()
        .devices()
        .filter(|(_, d)| {
            config
                .get_value()
                .resolve_integration(&d.integration)
                .is_some_and(|i| !functions(&i).is_empty())
        })
        .map(|(r, d)| (d.id.to_string(), format!("{} · {}", r.name, d.name)))
        .collect();
    view!{
        <h2 class="section">"Physical buttons"</h2>
        <p class="dim">"Choose a button, then assign a device and function. These overrides apply only while this activity is open. A long press waits 600 ms and does not also fire the short action. For example, control Kodi with the D-pad and your receiver with the volume keys."</p>
        <section class="card button-editor">
            <div class="remote-map" aria-label="Remote button layout">
                <div class="remote-display"><strong>"couch."</strong><span>"Select a button"</span></div>
                <div class="remote-dpad"></div>
                {KEYS.iter().map(move |&(button,name,glyph,x,y)|view!{
                    <button type="button" class="remote-key" class:chosen=move ||selected.get()==button class:mapped=activity.get_value().buttons.iter().any(|b|b.button==button)
                        style=format!("left:{x}%;top:{y}%") aria-label=name aria-pressed=move ||(selected.get()==button).to_string()
                        title=format!("{name}: {}",mapping_label(button)) on:click=move |_|load(button)>{glyph}</button>
                }).collect_view()}
            </div>
            <div class="button-assignment">
                <h3>{move ||KEYS.iter().find(|k|k.0==selected.get()).map(|k|k.1).unwrap_or("Button")}</h3>
                <p class="dim">{move ||format!("Saved: {}",mapping_label(selected.get()))}</p>
                <label class="field">"Press type"<select aria-label="Press type" prop:value=move ||if gesture.get()==Gesture::Long {"long"}else{"short"} on:change=move |e|{gesture.set(if event_target_value(&e)=="long" {Gesture::Long}else{Gesture::Short});load(selected.get_untracked());}>
                    <option value="short">"Short press"</option><option value="long" disabled=move ||!selected.get().supports_long()>"Long press (600 ms)"</option>
                </select></label>
                <label class="field">"Behavior"<select aria-label="Button behavior" prop:value=move ||mode.get() on:change=move |e|mode.set(event_target_value(&e))>
                    <option value="default">"Activity default"</option><option value="device">"Device function"</option><option value="disabled">"Do nothing"</option>
                </select></label>
                {move ||(mode.get()=="device").then(||view!{
                    <label class="field">"Device"<select aria-label="Button device" prop:value=move ||device.get() on:change=move |e|{device.set(event_target_value(&e));command.set(String::new());}>
                        <option value="">"Choose a device"</option>{options.clone().into_iter().map(|(id,name)|view!{<option value=id>{name}</option>}).collect_view()}
                    </select></label>
                    <label class="field">"Function"<select aria-label="Button function" prop:value=move ||command.get() on:change=move |e|command.set(event_target_value(&e))>
                        <option value="">"Choose a function"</option>{move ||{
                            let cfg=config.get_value();
                            let choices=cfg.devices().find(|(_,d)|d.id.as_str()==device.get()).and_then(|(_,d)|cfg.resolve_integration(&d.integration)).map(|i|functions(&i)).unwrap_or(&[]).iter().map(|&(id,name)|view!{<option value=id>{name}</option>}).collect_view(); choices
                        }}
                    </select></label>
                })}
                <button class="primary" disabled=move ||app.busy.get() || mode.get()=="device" && (device.get().is_empty()||command.get().is_empty()) on:click=move |_|{
                    let mut next=activity.get_value();next.buttons.retain(|b|b.button!=selected.get_untracked() || b.gesture!=gesture.get_untracked());
                    match mode.get_untracked().as_str(){
                        "device"=>next.buttons.push(Binding{button:selected.get_untracked(),gesture:gesture.get_untracked(),action:Some(Action::new(Id::new(device.get_untracked()),command.get_untracked()))}),
                        "disabled"=>next.buttons.push(Binding{button:selected.get_untracked(),gesture:gesture.get_untracked(),action:None}),_=>{}
                    }
                    app.run(api::put(format!("/api/activities/{}",next.id),next));
                }>"Save button mapping"</button>
                <p class="dim">"Saving does not send a command. Reopen the activity on the remote to load changes. The on-screen back arrow always returns to Couch. Infrared functions will appear when IR sending is available."</p>
            </div>
        </section>
        <details class="card"><summary>"All button mappings"</summary><ul class="rows">{KEYS.iter().map(move |&(button,name,_,_,_)|view!{
            <li class="row"><button class="row-main" on:click=move |_|load(button)><span class="row-title">{name}</span><span class="row-sub">{move ||format!("{}: {}",if gesture.get()==Gesture::Long {"Long press"}else{"Short press"},mapping_label(button))}</span></button></li>
        }).collect_view()}</ul></details>
    }.into_any()
}
