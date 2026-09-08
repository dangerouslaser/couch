#!/bin/sh
# Boot Android once, from wherever we are.
#
# Which slot Android lives in depends on whether the slots have been swapped, so
# the BCB has the opposite meaning in each layout.
#
# Since the kernel work the recovery slot holds the Couch recovery image, not
# Android, so in the linux-in-boot layout this script would land there instead.
# Android's boot image is only in the backup: flash android-p9-BACKUP.img to p9
# first, then run this with ANDROID_ON_P9=1.
cd "$(dirname "$0")/.."
if [ "$(cat build/layout 2>/dev/null)" = "linux-in-boot" ] && [ -z "$ANDROID_ON_P9" ]; then
    echo "p9 holds the Couch recovery image, not Android; see the comment in this script" >&2
    exit 1
fi
. tools/env.sh
LAYOUT=$(cat build/layout 2>/dev/null || echo android-in-boot)

if [ "$LAYOUT" = "linux-in-boot" ]; then
    # Android is in the recovery slot: ask lk for it.
    python3 tools/sercmd.py 'printf "boot-recovery" | dd of=/dev/mmcblk0p10 bs=512 count=1 conv=notrunc 2>/dev/null; sync; reboot -f' >/dev/null 2>&1
else
    # Android is already the normal boot target: a plain reboot is enough.
    python3 tools/sercmd.py 'sync; reboot -f' >/dev/null 2>&1
fi
echo "booting Android..."
i=0
while [ $i -lt 40 ]; do
    $ADB devices 2>/dev/null | grep -q "device$" && {
        # Do not let a stale flag send us round again on the next reboot.
        $ADB root >/dev/null 2>&1 || true; sleep 2
        $ADB shell 'dd if=/dev/zero of=/dev/block/mmcblk0p10 bs=512 count=1 2>/dev/null; sync' 2>/dev/null
        echo "android up (BCB cleared)"; exit 0; }
    sleep 3; i=$((i+1))
done
echo "android did not appear"; exit 1
