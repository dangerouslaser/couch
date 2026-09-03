#!/bin/sh
# Write the built image to the RECOVERY slot. Android's boot path is never touched.
set -e
cd "$(dirname "$0")/.."
. tools/env.sh
BB=/data/local/tmp/busybox

# Flashing needs Android. If we are in Linux there is no adb, so go via the
# serial shell instead of timing out on a device that is up and healthy.
if ! $ADB devices 2>/dev/null | grep -q "device$"; then
    if ls /dev/cu.usbmodem* >/dev/null 2>&1; then
        echo "in Linux; rebooting to Android to flash ..."
        sh "$(dirname "$0")/boot-android.sh" >/dev/null 2>&1 || true
    fi
fi
sh "$(dirname "$0")/wait-adb.sh" 60 || exit 1

# Android's toybox dd has conv= disabled; push busybox once and use that instead.
$ADB shell "[ -x $BB ]" 2>/dev/null || {
    $ADB push build/busybox-armv7l $BB >/dev/null
    $ADB shell "chmod 755 $BB"
}

$ADB push build/linux-recovery.img /data/local/tmp/lr.img >/dev/null
# Clear the marker area on EXPDB, never on the slot holding the boot image.
# These once shared a partition, and when the markers moved to expdb the sector
# offset became 0 - so this line wiped the image it had just written, leaving a
# zeroed slot that lk silently skips.
$ADB shell "$BB dd if=/data/local/tmp/lr.img of=$PART_RECOVERY bs=1048576 conv=notrunc 2>&1 | tail -1
$BB dd if=/dev/zero of=$PART_EXPDB bs=512 seek=$MARKER_SECTOR count=64 conv=notrunc 2>&1 | tail -1
sync"

# Refuse to leave a slot that will not boot.
MAGIC=$($ADB exec-out "dd if=$PART_RECOVERY bs=8 count=1" 2>/dev/null)
[ "$MAGIC" = "ANDROID!" ] || { echo "FLASH FAILED: recovery slot does not start with ANDROID! (got '$MAGIC')"; exit 1; }
echo "verified: recovery slot holds a valid boot image"
echo "flashed to recovery"
