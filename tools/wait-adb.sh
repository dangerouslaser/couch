#!/bin/sh
# Wait for Android over adb, with a deadline.
#
# "adb wait-for-device" blocks forever if the device is absent - powered off,
# unplugged, or sitting in the bootloader menu - which is indistinguishable from
# a slow boot and hangs any script that calls it.
cd "$(dirname "$0")/.."
. tools/env.sh
LIMIT=${1:-60}
DEADLINE=$(( $(date +%s) + LIMIT ))
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
    if $ADB devices 2>/dev/null | grep -q "device$"; then
        $ADB root >/dev/null 2>&1 || true
        sleep 2
        $ADB devices 2>/dev/null | grep -q "device$" && exit 0
    fi
    sleep 1
done
echo "TIMEOUT: no adb device after ${LIMIT}s - is the remote powered on and plugged in?" >&2
exit 1
