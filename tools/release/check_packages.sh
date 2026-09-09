#!/bin/sh
# Offline signature and dependency checks only; never executes package scripts.
set -eu
OUT=/out
APK_ARCH=${APK_ARCH:-armv7}
case "$APK_ARCH" in armv7|x86_64) ;; *) exit 2 ;; esac
apk --repositories-file /dev/null --keys-dir /usr/share/apk/keys/"$APK_ARCH" verify "$OUT"/packages/*.apk
mkdir -p /tmp/offline/etc/apk /tmp/offline/lib/apk/db
touch /tmp/offline/lib/apk/db/installed /tmp/offline/etc/apk/world
# force-non-repository permits explicit local APKs in an empty temporary
# database; signature/dependency checks remain enabled. Simulation is essential:
# ARM binaries, maintainer scripts and triggers must not execute on the builder.
apk --root /tmp/offline --arch "$APK_ARCH" --keys-dir /usr/share/apk/keys/"$APK_ARCH" \
    --repositories-file /dev/null --no-network --initdb --no-scripts \
    add --simulate --force-non-repository "$OUT"/packages/*.apk
