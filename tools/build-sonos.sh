#!/bin/sh
# Build the standalone Sonos client included in the runtime inventory.
set -eu
cd "$(dirname "$0")/.."
TARGET=${TARGET:-armv7-unknown-linux-musleabihf}
case "${1:-}" in
    --host) TARGET=host ;;
    '') ;;
    *) echo 'Usage: tools/build-sonos.sh [--host]' >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || { echo 'Usage: tools/build-sonos.sh [--host]' >&2; exit 2; }
if [ "$TARGET" = host ]; then
    (cd clients && cargo build --locked --release -p couch-sonos)
    SONOS_BIN=clients/target/release/couch-sonos
else
    # rust-lld links the final binary. The Sonos Control API is HTTPS, so ring's
    # crypto primitives are compiled too, for which macOS needs an ARM musl C compiler.
    if [ "$(uname -s)" = Darwin ] && [ -z "${CC_armv7_unknown_linux_musleabihf:-}" ]; then
        ZIG=${ZIG:-$(pwd)/build/toolchains/zig-aarch64-macos-0.15.2/zig}
        [ -x "$ZIG" ] || ZIG=$(command -v zig || true)
        [ -n "$ZIG" ] || { echo "Install Zig for the ARM HTTPS crypto build (or set CC_armv7_unknown_linux_musleabihf)."; exit 1; }
        export ZIG
        export CC_armv7_unknown_linux_musleabihf="$(pwd)/tools/arm-musl-cc.py"
    fi
    # Run inside clients so its static ARM linker configuration is applied.
    (cd clients && cargo build --locked --release --target "$TARGET" -p couch-sonos)
    SONOS_BIN=clients/target/$TARGET/release/couch-sonos
fi
printf '%s (%s bytes)\n' "$SONOS_BIN" "$(wc -c < "$SONOS_BIN" | tr -d ' ')"
