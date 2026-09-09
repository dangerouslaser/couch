use std::path::PathBuf;

fn main() {
    let gui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui/couch-gui");
    let fonts = gui.join("fonts");
    println!("cargo:rerun-if-changed={}", fonts.display());
    // Use the exact device fonts and software-renderer image preparation.
    std::env::set_var("SLINT_FONT_PATH", &fonts);
    std::env::set_var("SLINT_DEFAULT_FONT", fonts.join("Lato-Regular.ttf"));
    let config = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config(gui.join("ui/app.slint"), config)
        .expect("compile the actual Couch Slint UI for the browser");
}
