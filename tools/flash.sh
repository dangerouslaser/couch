#!/bin/sh
# Write the built image to the RECOVERY slot. Android's boot path is never touched.
set -e
cd "$(dirname "$0")/.."
. tools/env.sh
BB=/data/local/tmp/busybox

sh "$(dirname "$0")/wait-adb.sh" 60 || exit 1

# Android's toybox dd has conv= disabled; push busybox once and use that instead.
$ADB shell "[ -x $BB ]" 2>/dev/null || {
    $ADB push build/busybox-armv7l $BB >/dev/null
    $ADB shell "chmod 755 $BB"
}

$ADB push build/linux-recovery.img /data/local/tmp/lr.img >/dev/null
$ADB shell "$BB dd if=/data/local/tmp/lr.img of=$PART_RECOVERY bs=1048576 conv=notrunc 2>&1 | tail -1
$BB dd if=/dev/zero of=$PART_RECOVERY bs=512 seek=$MARKER_SECTOR count=8192 conv=notrunc 2>&1 | tail -1
sync"
echo "flashed to recovery"
