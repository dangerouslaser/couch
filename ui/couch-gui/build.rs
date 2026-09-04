use std::collections::HashMap;
use std::path::PathBuf;

fn main() {
    // The face Slint embeds by default reads as technical for a device that
    // lives in a living room. Slint picks its font at compile time, so the
    // choice is made here; both weights are present so font-weight: 600
    // resolves to a real face instead of being synthesised.
    let fonts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts");
    println!("cargo:rerun-if-changed={}", fonts.display());
    unsafe {
        std::env::set_var("SLINT_FONT_PATH", &fonts);
        std::env::set_var("SLINT_DEFAULT_FONT", fonts.join("Lato-Regular.ttf"));
    }

    // Lucide as real vector paths rather than rasterised alpha masks. This
    // needs the "path" feature on both i-slint-core and the software renderer,
    // which the slint facade forwards from neither - hence the two direct
    // dependencies in Cargo.toml that exist only to switch it on.
    let libs = HashMap::from([("lucide".to_string(), PathBuf::from(lucide_slint::lib()))]);

    let cfg = slint_build::CompilerConfiguration::new()
        .with_library_paths(libs)
        // Embed fonts and images for the software renderer: there is no font
        // system on the device to fall back to.
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config("ui/app.slint", cfg).expect("compiling ui/app.slint");
}
