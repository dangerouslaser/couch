//! Browser preview of the production Slint component tree, with local fixtures.
//! There are no device clients, credentials, or hardware operations in this crate.
use slint::ComponentHandle;
use wasm_bindgen::prelude::*;

slint::include_modules!();
mod demo;

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let app = App::new().map_err(|error| JsValue::from_str(&error.to_string()))?;
    demo::configure(&app);
    app.run()
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// Route the demo's physical-button controls into local Rust fixture logic.
#[wasm_bindgen]
pub fn remote_button(name: &str) {
    demo::remote_button(name);
}

/// Local-only observable state for accessibility and browser regression checks.
#[wasm_bindgen]
pub fn state_json() -> String {
    demo::state_json()
}
