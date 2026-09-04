fn main() {
    // Embed fonts and images into the binary, rasterised for the software
    // renderer: there is no font system on the device to fall back to.
    let cfg = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config("ui/app.slint", cfg).expect("compiling ui/app.slint");
}
