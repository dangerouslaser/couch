#!/bin/sh
# Offline signature and dependency checks only; never executes package scripts.
set -eu
OUT=/out
apk --repositories-file /dev/null --keys-dir /usr/share/apk/keys/armv7 verify "$OUT"/packages/*.apk
mkdir -p /tmp/offline/etc/apk /tmp/offline/lib/apk/db
touch /tmp/offline/lib/apk/db/installed /tmp/offline/etc/apk/world
# force-non-repository permits explicit local APKs in an empty temporary
# database; signature/dependency checks remain enabled. Simulation is essential:
# ARM binaries, maintainer scripts and triggers must not execute on the builder.
apk --root /tmp/offline --arch armv7 --keys-dir /usr/share/apk/keys/armv7 \
    --repositories-file /dev/null --no-network --initdb --no-scripts \
    add --simulate --force-non-repository "$OUT"/packages/*.apk
