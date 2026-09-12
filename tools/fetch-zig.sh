#!/bin/sh
# Download the pinned Zig for this host into build/toolchains, verified against
# the SHA-256 that ziglang.org/download/index.json publishes for the release.
# tools/arm-musl-cc.py is written for exactly this version; the build scripts
# find the result through tools/arm-cc-env.sh. Safe to rerun: an installed
# copy is reported and left alone.
set -eu
cd "$(dirname "$0")/.."
VERSION=0.15.2
case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) HOST=aarch64-macos; SHA=3cc2bab367e185cdfb27501c4b30b1b0653c28d9f73df8dc91488e66ece5fa6b ;;
    Linux-x86_64) HOST=x86_64-linux; SHA=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239 ;;
    Linux-aarch64) HOST=aarch64-linux; SHA=958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f ;;
    *) echo "No pinned Zig $VERSION for $(uname -s)-$(uname -m)." >&2; exit 1 ;;
esac
DIR=build/toolchains
NAME=zig-$HOST-$VERSION
if [ -x "$DIR/$NAME/zig" ]; then
    echo "$DIR/$NAME/zig ($("$DIR/$NAME/zig" version)) already installed"
    exit 0
fi
mkdir -p "$DIR"
ARCHIVE=$DIR/$NAME.tar.xz
curl -fsSL -o "$ARCHIVE" "https://ziglang.org/download/$VERSION/$NAME.tar.xz"
if command -v sha256sum >/dev/null 2>&1; then
    echo "$SHA  $ARCHIVE" | sha256sum -c -
else
    echo "$SHA  $ARCHIVE" | shasum -a 256 -c -
fi
tar -xJf "$ARCHIVE" -C "$DIR"
rm -f "$ARCHIVE"
echo "$DIR/$NAME/zig ($("$DIR/$NAME/zig" version))"
