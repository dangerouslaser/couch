# Lucide icon assets

Vendored from the official `lucide-static` npm package, version 1.43.0.
Source: https://github.com/lucide-icons/lucide
Documentation: https://lucide.dev/guide/packages/lucide-static

All 2,077 SVGs are included. See LICENSE for Lucide/Feather attribution.
The web picker serves these locally, without a CDN or React dependency.

Run `python3 tools/build-icon-catalog.py` from the repository root to regenerate
`model/couch-model/src/icon_catalog.rs` and the device's 24px alpha atlas.
This requires Pillow and rsvg-convert. Commit the generated files together;
normal Rust builds do not download or rasterize icons.
