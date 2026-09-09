use crate::{api, App};
use leptos::{prelude::*, task::spawn_local};
use serde_json::json;

pub fn setup(app: App, connection: &couch_model::Connection) -> AnyView {
    let base = StoredValue::new(format!(
        "/api/connections/{}/kodi/connection",
        connection.id
    ));
    let port = RwSignal::new("8080".to_string());
    let username = RwSignal::new("kodi".to_string());
    let password = RwSignal::new(String::new());
    let password_set = RwSignal::new(false);
    let clear = RwSignal::new(false);
    let http = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    spawn_local(async move {
        match api::ha("GET", &base.get_value(), None).await {
            Ok(s) => {
                port.set(s["web_port"].to_string());
                username.set(s["username"].as_str().unwrap_or("kodi").into());
                password_set.set(s["password_set"].as_bool().unwrap_or(false));
                http.set(s["http_control"].as_bool().unwrap_or(false));
            }
            Err(e) => {
                if e.unauthorized {
                    app.paired.set(Some(false));
                }
                message.set(e.message);
            }
        }
    });
    view! {<section><h3>"Kodi web access"</h3>
        <p>"Enable ‘Allow remote control via HTTP’ in Kodi → Settings → Services → Control. These credentials also load fanart and logos."</p>
        {super::connections::field("HTTP port",port,"8080")}
        {super::connections::field("Username",username,"kodi")}
        <label class="field">"Password"<input type="password" autocomplete="new-password" prop:value=move ||password.get() placeholder=move ||if password_set.get(){"Saved — leave blank to keep"}else{"Enter your Kodi password"} on:input=move |e|password.set(event_target_value(&e))/></label>
        <label><input type="checkbox" prop:checked=move ||clear.get() on:change=move |e|clear.set(event_target_checked(&e))/>"Use an empty password"</label>
        <label class="field">"Control connection"<select aria-label="Control connection" prop:value=move ||if http.get(){"http"}else{"tcp"} on:change=move |e|http.set(event_target_value(&e)=="http")><option value="tcp">"TCP with push updates; credentials for artwork"</option><option value="http">"Authenticated HTTP API"</option></select></label>
        <p class="dim">"TCP uses the saved TCP port and has no login. HTTP uses this username and password for every command. Passwords stay private on the remote."</p>
        <button class="primary" disabled=move ||busy.get() on:click=move |_|{
            let Ok(port)=port.get_untracked().parse::<u16>() else {message.set("Enter a valid HTTP port".into());return;};
            let pass=password.get_untracked();
            let saved_password=if clear.get_untracked(){Some(String::new())}else if pass.is_empty() && password_set.get_untracked(){None}else{Some(pass)};
            let body=json!({"web_port":port,"username":username.get_untracked(),"password":saved_password,"http_control":http.get_untracked()});
            busy.set(true);message.set("Testing Kodi…".into());
            spawn_local(async move {match api::ha("PUT",&base.get_value(),Some(body)).await {
                Ok(s)=>{password.set(String::new());clear.set(false);password_set.set(s["password_set"].as_bool().unwrap_or(false));message.set("Connected and saved".into());},
                Err(e)=>{if e.unauthorized{app.paired.set(Some(false));}message.set(e.message);}
            }busy.set(false);});
        }>"Test & save Kodi credentials"</button><p role="status">{move ||message.get()}</p>
    </section>}.into_any()
}
