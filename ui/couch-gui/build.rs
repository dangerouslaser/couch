use std::path::PathBuf;

fn main() {
    // The default embedded face reads as rather technical for a device that
    // lives in a living room. Slint picks its font at compile time, so the
    // choice is made here: SLINT_FONT_PATH is the directory it searches and
    // SLINT_DEFAULT_FONT the face it starts from. Both weights are present so
    // font-weight: 600 has something to resolve to rather than being faked.
    let fonts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts");
    println!("cargo:rerun-if-changed={}", fonts.display());
    std::env::set_var("SLINT_FONT_PATH", &fonts);
    std::env::set_var("SLINT_DEFAULT_FONT", fonts.join("Lato-Regular.ttf"));

    // Embed fonts and images into the binary, rasterised for the software
    // renderer: there is no font system on the device to fall back to.
    let cfg = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config("ui/app.slint", cfg).expect("compiling ui/app.slint");
}
