use std::path::PathBuf;

fn main() {
    // couch-gui's fonts, embedded the way couch-gui embeds them, so the
    // harness rasterises the same glyphs at the same sizes the panel will.
    let fonts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../couch-gui/fonts");
    println!("cargo:rerun-if-changed={}", fonts.display());
    println!("cargo:rerun-if-changed=../couch-gui/ui");
    unsafe {
        std::env::set_var("SLINT_FONT_PATH", &fonts);
        std::env::set_var("SLINT_DEFAULT_FONT", fonts.join("Lato-Regular.ttf"));
    }

    let cfg = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config("ui/demo.slint", cfg).expect("compiling ui/demo.slint");
}
