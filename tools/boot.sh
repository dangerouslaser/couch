#!/bin/sh
# Reboot into Linux and claim the serial session before its 4-minute self-reboot.
set -e
cd "$(dirname "$0")/.."
. tools/env.sh

$ADB reboot recovery
echo "booting into Linux..."
# The MediaTek preloader briefly exposes its own CDC port around 6s; wait past it
# or you will connect to the bootloader instead of our gadget.
sleep 25
i=0
while [ $i -lt 40 ]; do
    if ls /dev/cu.usbmodem* >/dev/null 2>&1; then
        sleep 2
        echo "shell up: $(ls /dev/cu.usbmodem*)"
        python3 tools/sercmd.py 'touch /tmp/stay; echo CLAIMED'
        exit 0
    fi
    sleep 2; i=$((i+1))
done
echo "no serial port appeared; read the markers with tools/markers.sh after it reboots"
exit 1
