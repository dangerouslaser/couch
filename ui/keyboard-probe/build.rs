use std::path::PathBuf;

fn main() {
    let fonts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../couch-gui/fonts");
    println!("cargo:rerun-if-changed={}", fonts.display());
    println!("cargo:rerun-if-changed=../couch-gui/ui");
    unsafe {
        std::env::set_var("SLINT_FONT_PATH", &fonts);
        std::env::set_var("SLINT_DEFAULT_FONT", fonts.join("Lato-Regular.ttf"));
    }

    // Two .slint files exporting the same component, chosen by the feature, so
    // that only one of them is ever compiled into a binary. Selecting between
    // them inside one file would embed both.
    let ui = if std::env::var_os("CARGO_FEATURE_KEYBOARD").is_some() {
        "ui/with-keyboard.slint"
    } else if std::env::var_os("CARGO_FEATURE_TEXTINPUT").is_some() {
        "ui/textinput.slint"
    } else {
        "ui/baseline.slint"
    };
    let cfg = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config(ui, cfg).unwrap_or_else(|e| panic!("compiling {ui}: {e}"));
}
