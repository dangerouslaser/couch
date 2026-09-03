#!/bin/sh
# Reboot straight back into Linux, skipping Android entirely.
#
# lk reads the boot-control block at the start of para to decide what to boot,
# so writing "boot-recovery" there and rebooting lands us back in Linux. init
# clears the BCB on the way up, so it is a one-shot and a later reboot still
# returns to Android.
cd "$(dirname "$0")/.."
LAYOUT=$(cat build/layout 2>/dev/null || echo android-in-boot)

# If there is no serial port we are in Android, not Linux: go via adb instead.
if ! ls /dev/cu.usbmodem* >/dev/null 2>&1; then
    exec sh "$(dirname "$0")/boot.sh"
fi
if [ "$LAYOUT" = "linux-in-boot" ]; then
    # Linux is the normal boot target: clear the flag and just reboot.
    python3 tools/sercmd.py 'dd if=/dev/zero of=/dev/mmcblk0p10 bs=512 count=1 conv=notrunc 2>/dev/null; sync; reboot -f' >/dev/null 2>&1
else
    python3 tools/sercmd.py 'printf "boot-recovery" | dd of=/dev/mmcblk0p10 bs=512 count=1 conv=notrunc 2>/dev/null; sync; reboot -f' >/dev/null 2>&1
fi
echo "rebooting straight back into Linux..."
sh "$(dirname "$0")/wait-shell.sh" 90 || exit 1
python3 tools/sercmd.py 'touch /tmp/stay; echo CLAIMED'
