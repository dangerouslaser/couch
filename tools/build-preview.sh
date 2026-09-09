#!/bin/sh
# Compile the actual device Slint component tree with local browser fixtures.
set -eu
cd "$(dirname "$0")/.."

BINDGEN_VERSION=0.2.127
BINDGEN=${WASM_BINDGEN:-}
if [ -z "$BINDGEN" ]; then
    BINDGEN=$(command -v wasm-bindgen || true)
fi
if [ -z "$BINDGEN" ] && [ -x "$HOME/Library/Caches/dev.trunkrs.trunk/wasm-bindgen-$BINDGEN_VERSION/wasm-bindgen" ]; then
    BINDGEN="$HOME/Library/Caches/dev.trunkrs.trunk/wasm-bindgen-$BINDGEN_VERSION/wasm-bindgen"
fi
if [ -z "$BINDGEN" ] || [ "$("$BINDGEN" --version)" != "wasm-bindgen $BINDGEN_VERSION" ]; then
    echo "Install the matching browser binding generator:"
    echo "  cargo install wasm-bindgen-cli --version $BINDGEN_VERSION --locked"
    echo "Or set WASM_BINDGEN to its executable path."
    exit 1
fi
rustup target list --installed | grep -qx wasm32-unknown-unknown || {
    echo "Install the browser Rust target: rustup target add wasm32-unknown-unknown"
    exit 1
}
cargo build --manifest-path preview/Cargo.toml --locked --release --target wasm32-unknown-unknown
mkdir -p site/wasm
"$BINDGEN" preview/target/wasm32-unknown-unknown/release/couch_preview.wasm \
    --target web --out-dir site/wasm --out-name couch_preview
cp preview/licenses/LATO-OFL.txt preview/licenses/LUCIDE-LICENSE site/wasm/
printf 'Slint browser preview built: site/wasm/couch_preview.js\n'
