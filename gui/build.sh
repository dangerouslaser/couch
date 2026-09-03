#!/bin/sh
# Build the LVGL app statically with the Android NDK.
#
# Cross-compiling on the host, not on the device: the cache partition is 112MB
# and gcc alone does not fit (it filled the disk and failed). A static binary
# needs no libc at runtime, so a bionic-linked static build runs fine on Alpine -
# same trick as fbcon.
set -e
cd "$(dirname "$0")"
NDK="${NDK:-$HOME/Library/Android/sdk/ndk/29.0.14206865}"
CC="$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/armv7a-linux-androideabi21-clang"
STRIP="$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-strip"
LVGL=../build/lvgl

[ -x "$CC" ] || { echo "no NDK clang at $CC"; exit 1; }
[ -d "$LVGL/src" ] || { echo "no lvgl checkout at $LVGL"; exit 1; }

# Backends for other platforms are dead weight here; LVGL guards them by config
# but they still cost compile time.
SRCS=$(find "$LVGL/src" -name '*.c' \
    ! -path '*/drivers/glfw/*'    ! -path '*/drivers/sdl/*' \
    ! -path '*/drivers/wayland/*' ! -path '*/drivers/x11/*' \
    ! -path '*/drivers/windows/*' ! -path '*/drivers/nuttx/*' \
    ! -path '*/drivers/qnx/*'     ! -path '*/drivers/libinput/*')

echo "compiling $(echo "$SRCS" | wc -l | tr -d ' ') lvgl sources + main.c ..."
"$CC" -static -Os -DLV_CONF_INCLUDE_SIMPLE \
    -I. -I../build -I"$LVGL" \
    -ffunction-sections -fdata-sections -Wl,--gc-sections \
    -o couch-gui main.c theme.c icons.c $SRCS -lm
"$STRIP" couch-gui
echo "couch-gui: $(stat -f%z couch-gui) bytes"
