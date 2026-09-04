#!/bin/sh
# Build the voice client for the remote, and print what it costs.
#
# The device target is the default because that is the artefact that matters;
# --host builds the same thing for this machine, which is what runs the tests
# and what talks to a hub from a laptop.
#
# Sizes are printed for the same reason tools/build-webui.sh prints them: every
# byte here lives on the device's flash, and a build that quietly doubled is
# worth seeing at the end of every run.
set -e
cd "$(dirname "$0")/.."

TARGET=${TARGET:-armv7-unknown-linux-musleabihf}
[ "$1" = "--host" ] && TARGET=host

if [ "$TARGET" = host ]; then
    ( cd clients && cargo build --release -p couch-voice )
    DIR=clients/target/release
else
    # rust-lld, not a cross-gcc: see clients/.cargo/config.toml. This is also
    # why the crate binds no C library - an ALSA client that linked libasound
    # would need a sysroot none of this has.
    rustup target list --installed | grep -qx "$TARGET" || {
        echo "no $TARGET toolchain: rustup target add $TARGET"; exit 1; }
    ( cd clients && cargo build --release --target "$TARGET" -p couch-voice )
    DIR=clients/target/$TARGET/release
fi

echo
for b in couch-voice couch-mic; do
    printf "  %-46s %8s\n" "$b ($TARGET)" "$(wc -c < "$DIR/$b" | tr -d ' ')"
done
echo "  $DIR"
