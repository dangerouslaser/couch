//! The handful of controls every screen is built from.
//!
//! Plain functions returning views rather than `#[component]`s: they are
//! called from inside closures that already own the data, and taking owned
//! values as arguments is less ceremony than a props struct for a two-line
//! widget.

use std::rc::Rc;

use couch_model::{Icon, ALL_ICONS};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::route::Route;
use crate::App;

/// A screen title, with an optional way back up the tree.
///
/// The browser's own Back does the same thing - the router pushes real history
/// entries - but a visible affordance is not optional on a phone in a kiosk-ish
/// context where the chrome may be hidden.
pub fn page_header(app: App, title: String, back: Option<Route>) -> AnyView {
    view! {
        <div class="page-head">
            {back.map(|route| view! {
                <button aria-label="Back to list" class="back" on:click=move |_| app.go(route.clone())>"‹"</button>
            })}
            <h1>{title}</h1>
        </div>
    }
    .into_any()
}

/// A text input that commits when the user leaves it or presses Enter.
///
/// Not on input: every commit is a round trip and an fsync on flash, so
/// per-keystroke saves would be both slow and a way to wear the partition out
/// spelling "Living room".
pub fn text_field(
    label: &'static str,
    value: String,
    placeholder: &'static str,
    commit: impl Fn(String) + 'static,
) -> AnyView {
    let original = value.clone();
    view! {
        <label class="field">
            <span class="label">{label}</span>
            <input
                type="text"
                value=value
                placeholder=placeholder
                on:change=move |ev| {
                    let next = event_target_value(&ev);
                    // A no-op edit would still bump the revision and write the
                    // file, which then invalidates anyone else's If-Match.
                    if next.trim() != original.trim() && !next.trim().is_empty() {
                        commit(next.trim().to_string());
                    }
                }
                on:keydown=move |ev| {
                    if ev.key() == "Enter" {
                        // Blur, and let the change handler above do the work.
                        let _ = ev.target()
                            .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
                            .map(|el| el.blur());
                    }
                }
            />
        </label>
    }
    .into_any()
}

/// Icons by name.
///
/// The device draws these as rasterised Lucide PNGs, which this page
/// deliberately does not carry: the remote serves the config UI on a LAN that
/// may have no route to the internet, so anything shown here has to be in the
/// bundle, and fifteen icons is a sprite sheet to keep in step with
/// `tools/mkuiicons.sh` for a picker used once per room. The list comes from
/// `couch-model`, so it cannot drift from what the device can actually render.
pub fn icon_select(current: Option<Icon>, commit: impl Fn(Option<Icon>) + 'static) -> AnyView {
    let selected = current.map(|i| i.name().to_string()).unwrap_or_default();
    view! {
        <label class="field">
            <span class="label">"Icon"</span>
            <select on:change=move |ev| {
                let name = event_target_value(&ev);
                commit(Icon::from_name(&name));
            }>
                <option value="" selected=selected.is_empty()>"Automatic"</option>
                {ALL_ICONS
                    .iter()
                    .map(|icon| {
                        let name = icon.name();
                        view! {
                            <option value=name selected=selected == name>{name}</option>
                        }
                    })
                    .collect_view()}
            </select>
        </label>
    }
    .into_any()
}

/// A destructive button that asks first.
///
/// Two taps on the same control rather than a modal: `window.confirm` blocks
/// the wasm event loop and looks like the browser, not the app, and a dialog
/// component is a lot of machinery for "are you sure".
pub fn danger_button(label: &'static str, confirm: impl Fn() + 'static) -> AnyView {
    let armed = RwSignal::new(false);
    view! {
        <button
            type="button"
            class="danger"
            class:armed=move || armed.get()
            on:click=move |_| {
                if armed.get() {
                    armed.set(false);
                    confirm();
                } else {
                    armed.set(true);
                }
            }
            on:blur=move |_| armed.set(false)
        >
            {move || if armed.get() { "Confirm delete".to_string() } else { label.to_string() }}
        </button>
    }
    .into_any()
}

/// A single-field "add a thing" row.
pub fn add_row(
    placeholder: &'static str,
    button: &'static str,
    submit: impl Fn(String) + 'static,
) -> AnyView {
    let draft = RwSignal::new(String::new());
    // Two handlers need the same callback, and a closure capturing a non-Clone
    // `submit` is not itself Clone.
    let submit: Rc<dyn Fn(String)> = Rc::new(submit);
    let on_enter = submit.clone();

    fn flush(draft: RwSignal<String>, submit: &dyn Fn(String)) {
        let name = draft.get().trim().to_string();
        if !name.is_empty() {
            submit(name);
        }
    }

    view! {
        <div class="add-row">
            <input
                type="text"
                placeholder=placeholder
                aria-label=placeholder
                prop:value=move || draft.get()
                on:input=move |ev| draft.set(event_target_value(&ev))
                on:keydown=move |ev| {
                    if ev.key() == "Enter" {
                        flush(draft, &*on_enter);
                    }
                }
            />
            <button class="primary" disabled=move || draft.get().trim().is_empty() on:click=move |_| flush(draft, &*submit)>{button}</button>
        </div>
    }
    .into_any()
}

/// An empty-state line, so a list that is legitimately empty does not read as
/// a page that failed to load.
pub fn empty(message: &'static str) -> AnyView {
    view! { <p class="dim empty">{message}</p> }.into_any()
}
