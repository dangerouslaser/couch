#!/bin/sh
# Build the standalone Sonos client included in the runtime inventory.
set -eu
cd "$(dirname "$0")/.."
. tools/sonos-build-env.sh
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
    # rust-lld links the final binary; ring's C needs the ARM musl compiler
    # that tools/arm-cc-env.sh provides on macOS and Linux alike.
    . tools/arm-cc-env.sh
    # Run inside clients so its static ARM linker configuration is applied.
    (cd clients && cargo build --locked --release --target "$TARGET" -p couch-sonos)
    SONOS_BIN=clients/target/$TARGET/release/couch-sonos
fi
printf '%s (%s bytes)\n' "$SONOS_BIN" "$(wc -c < "$SONOS_BIN" | tr -d ' ')"
