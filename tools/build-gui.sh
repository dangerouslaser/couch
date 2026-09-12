#!/bin/sh
# Build the device GUI that the runtime inventory ships as couch-gui.
#
# ui/.cargo/config.toml already pins the linker and the Cortex-A7 target CPU;
# this script adds what cargo alone cannot: the Sonos developer key compiled
# into the binary, and the ARM musl C compiler that ring needs on macOS. The
# device target is the default because that is the artefact that matters;
# --host builds the same thing for this machine.
set -eu
cd "$(dirname "$0")/.."
. tools/sonos-build-env.sh
TARGET=${TARGET:-armv7-unknown-linux-musleabihf}
case "${1:-}" in
    --host) TARGET=host ;;
    '') ;;
    *) echo 'Usage: tools/build-gui.sh [--host]' >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || { echo 'Usage: tools/build-gui.sh [--host]' >&2; exit 2; }
if [ "$TARGET" = host ]; then
    (cd ui && cargo build --locked --release -p couch-gui)
    GUI_BIN=ui/target/release/couch-gui
else
    # rust-lld links the final binary. The GUI's HTTPS clients build ring's
    # crypto primitives, for which macOS needs an ARM musl C compiler.
    if [ "$(uname -s)" = Darwin ] && [ -z "${CC_armv7_unknown_linux_musleabihf:-}" ]; then
        ZIG=${ZIG:-$(pwd)/build/toolchains/zig-aarch64-macos-0.15.2/zig}
        [ -x "$ZIG" ] || ZIG=$(command -v zig || true)
        [ -n "$ZIG" ] || { echo "Install Zig for the ARM HTTPS crypto build (or set CC_armv7_unknown_linux_musleabihf)."; exit 1; }
        export ZIG
        export CC_armv7_unknown_linux_musleabihf="$(pwd)/tools/arm-musl-cc.py"
    fi
    # Run inside ui so its linker and target-cpu configuration is applied.
    (cd ui && cargo build --locked --release --target "$TARGET" -p couch-gui)
    GUI_BIN=ui/target/$TARGET/release/couch-gui
fi
printf '%s (%s bytes)\n' "$GUI_BIN" "$(wc -c < "$GUI_BIN" | tr -d ' ')"
