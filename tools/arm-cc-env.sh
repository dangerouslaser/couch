# Sourced by the build scripts from the repository root before an ARM cross-build.
#
# rust-lld links the static musl binaries, but crates that compile C (ring, for
# every HTTPS client) still need an ARM musl C compiler, and neither macOS nor
# a stock Linux host has one. The repository carries a Zig wrapper instead:
# tools/fetch-zig.sh downloads the pinned Zig into build/toolchains, and this
# file points cc-rs at it through tools/arm-musl-cc.py. A host that already
# exports CC_armv7_unknown_linux_musleabihf keeps its own compiler.
if [ -z "${CC_armv7_unknown_linux_musleabihf:-}" ]; then
    case "$(uname -s)-$(uname -m)" in
        Darwin-arm64) ZIG_HOST=aarch64-macos ;;
        Linux-x86_64) ZIG_HOST=x86_64-linux ;;
        Linux-aarch64) ZIG_HOST=aarch64-linux ;;
        *) ZIG_HOST=unknown ;;
    esac
    ZIG=${ZIG:-$(pwd)/build/toolchains/zig-$ZIG_HOST-0.15.2/zig}
    [ -x "$ZIG" ] || ZIG=$(command -v zig || true)
    [ -n "$ZIG" ] || {
        echo 'No Zig for the ARM C build: run tools/fetch-zig.sh (or set CC_armv7_unknown_linux_musleabihf).' >&2
        exit 1
    }
    export ZIG
    export CC_armv7_unknown_linux_musleabihf="$(pwd)/tools/arm-musl-cc.py"
fi
