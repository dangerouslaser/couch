//! The Couch config UI.
//!
//! A client-side Leptos app, served by `couch-confd` off the remote itself.
//! It shares `couch-model` with that daemon, so the shapes it edits are the
//! same types the device stores - the pickers for icons and device kinds are
//! built from the model's own lists rather than from a copy that can go stale.
//!
//! Two things shape the whole design:
//!
//! * **The config is one signal.** Every mutating call answers with the
//!   complete document, so there is no local patching and no way for the screen
//!   to disagree with the file. A house is a few KB; this is cheaper than the
//!   bugs the alternative buys.
//!
//! * **Edits commit on `change`, not on every keystroke.** Each commit is an
//!   fsync and a rename on a phone flash partition, and a per-character write
//!   would be both slow and pointless.

mod api;
mod route;
mod screens;
mod ui;

use std::future::Future;

use couch_model::Config;
use leptos::prelude::*;
use leptos::task::spawn_local;

use api::ApiError;
use route::{Route, Router};

/// Everything the screens share, passed by context.
///
/// `Copy` because Leptos signals are handles into an arena, so this can be
/// captured by every closure in the tree without a clone dance.
#[derive(Clone, Copy)]
pub struct App {
    pub config: RwSignal<Option<Config>>,
    pub error: RwSignal<Option<String>>,
    pub busy: RwSignal<bool>,
    pub router: Router,
}

impl App {
    /// Run one API call: mark the app busy, then either adopt the config it
    /// returns or show why it did not happen.
    ///
    /// Nothing is applied optimistically. On a device where a write can fail
    /// for real - a full partition, a config another client just changed -
    /// showing the edit and taking it back is worse than a half-second wait.
    pub fn run<F>(&self, call: F)
    where
        F: Future<Output = Result<Config, ApiError>> + 'static,
    {
        let (config, error, busy) = (self.config, self.error, self.busy);
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match call.await {
                Ok(next) => config.set(Some(next)),
                Err(e) => {
                    error.set(Some(e.message));
                    // The screen is now showing something the daemon refused
                    // to act on - most often a room that another phone has
                    // already deleted. Re-reading costs one GET and is the
                    // difference between a page that stays wrong and one that
                    // explains itself. A failed re-read changes nothing: the
                    // message from the edit is the more useful of the two.
                    if let Ok(fresh) = api::load().await {
                        config.set(Some(fresh));
                    }
                }
            }
            busy.set(false);
        });

    }

    pub fn go(&self, route: Route) {
        self.router.go(route);
    }
}

fn main() {
    install_panic_hook();
    mount_to_body(Shell);
}

/// A panic in wasm is otherwise an "unreachable executed" with no location.
/// Hooking it costs nothing at runtime and is the difference between a
/// debuggable report from someone's phone and a shrug.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("couch-web panic: {info}").into());
    }));
}

#[component]
fn Shell() -> impl IntoView {
    let app = App {
        config: RwSignal::new(None),
        error: RwSignal::new(None),
        busy: RwSignal::new(false),
        router: Router::install(),
    };
    provide_context(app);

    // The first load, and the only place the app fetches without being asked.
    app.run(api::load());

    let route = app.router.current;

    view! {
        <header class="bar">
            <span class="brand">"Couch"</span>
            <span class="spacer"></span>
            <span class="status">
                {move || if app.busy.get() { "saving…" } else { "" }}
            </span>
        </header>

        <main>
            // Inside main rather than over it: a fixed banner covered the
            // heading of whatever screen raised the error, which is the one
            // thing on the page that says what the message is about. It is
            // sticky instead, so it still follows a list being scrolled.
            {move || app.error.get().map(|message| view! {
                <div class="banner" role="alert">
                    <span>{message}</span>
                    <button class="link" on:click=move |_| app.error.set(None)>"dismiss"</button>
                </div>
            })}

            {move || match app.config.get() {

                None => view! { <p class="dim pad">"Loading the house…"</p> }.into_any(),
                Some(config) => screens::render(app, &config, route.get()),
            }}
        </main>

        <nav class="tabs">
            {[
                (Route::Areas, "Areas"),
                (Route::Scenes, "Scenes"),
                (Route::Activities, "Activities"),
            ]
                .into_iter()
                .map(|(target, label)| {
                    let for_class = target.clone();
                    let for_click = target.clone();
                    view! {
                        <button
                            class="tab"
                            class:on=move || route.get().tab() == for_class
                            on:click=move |_| app.go(for_click.clone())
                        >
                            {label}
                        </button>
                    }
                })
                .collect_view()}
        </nav>
    }
}
