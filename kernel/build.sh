#!/bin/sh
# Build the from-source kernel in the couch-kbuild container.
#
# Expects a checkout of the kernel tree (branch couch-ha100, see README.md) at
# $KTREE and puts the build output in $KOUT, so the source tree stays clean and
# a fresh build is `rm -rf $KOUT`. The config is kernel/couch-ha100.config,
# copied in on first build; edit it with `make ... menuconfig` in the same
# container and copy it back into the repo when it changes.
#
#   docker build -t couch-kbuild kernel/     # once
#   KTREE=~/couch-kernel/base kernel/build.sh
#
# Produces $KOUT/arch/arm/boot/zImage. Pack it with kernel/pack.py.
set -e
cd "$(dirname "$0")/.."
KTREE=${KTREE:-$HOME/couch-kernel/base}
KOUT=${KOUT:-$(dirname "$KTREE")/out}
[ -f "$KTREE/Makefile" ] || { echo "no kernel tree at $KTREE"; exit 1; }
mkdir -p "$KOUT"
[ -f "$KOUT/.config" ] || cp kernel/couch-ha100.config "$KOUT/.config"
docker run --rm -v "$KTREE":/src -v "$KOUT":/out -w /src couch-kbuild sh -c '
    make O=/out ARCH=arm CROSS_COMPILE=arm-eabi- olddefconfig >/out/config.log 2>&1 \
        || { echo CONFIG FAILED; tail -5 /out/config.log; exit 1; }
    make O=/out ARCH=arm CROSS_COMPILE=arm-eabi- -j"$(nproc)" zImage'
ls -la "$KOUT/arch/arm/boot/zImage"
