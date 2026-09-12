#!/bin/sh
# The config UI, and the daemon that carries it.
#
# The order is the whole reason this is a script. couch-confd's build.rs bakes
# web/couch-web/dist into the binary, so trunk has to have run first or the
# daemon ships the placeholder page that says it has no UI in it. Cargo cannot
# express that dependency - the two live in different workspaces, for different
# target triples, and only one of them is even built by cargo alone.
#
# The device target is the default because that is the artefact that matters;
# --host builds the same thing for this machine, which is what
# tools/run-webui.sh serves.
set -e
cd "$(dirname "$0")/.."
# The daemon links couch-sonos, so it carries the developer key too.
. tools/sonos-build-env.sh

TARGET=${TARGET:-armv7-unknown-linux-musleabihf}
[ "$1" = "--host" ] && TARGET=host

command -v trunk >/dev/null || {
    echo "no trunk on PATH. It builds the wasm bundle:"
    echo "  cargo install --locked trunk"
    echo "  rustup target add wasm32-unknown-unknown"
    exit 1
}

echo "= web/couch-web -> dist"
( cd web/couch-web && trunk build --release )

if [ "$TARGET" = host ]; then
    echo "= daemon/couch-confd -> host"
    ( cd daemon && cargo build --release )
    BIN=daemon/target/release/couch-confd
else
    # rust-lld links the final binary; ring's C needs the ARM musl compiler
    # that tools/arm-cc-env.sh provides on macOS and Linux alike.
    . tools/arm-cc-env.sh
    rustup target list --installed | grep -qx "$TARGET" || {
        echo "no $TARGET toolchain: rustup target add $TARGET"; exit 1; }
    echo "= daemon/couch-confd -> $TARGET"
    ( cd daemon && cargo build --release --target "$TARGET" )
    BIN=daemon/target/$TARGET/release/couch-confd
fi

# The sizes are the point of the exercise: everything here is downloaded over
# the remote's own WiFi or stored on its flash, so a build that quietly doubled
# is worth seeing at the end of every run.
echo
for f in web/couch-web/dist/*; do
    [ -f "$f" ] || continue
    printf "  %-46s %7s  %7s gz\n" "$(basename "$f")" \
        "$(wc -c < "$f" | tr -d ' ')" "$(gzip -9 -c "$f" | wc -c | tr -d ' ')"
done
printf "  %-46s %7s\n" "couch-confd ($TARGET)" "$(wc -c < "$BIN" | tr -d ' ')"
echo "  $BIN"
