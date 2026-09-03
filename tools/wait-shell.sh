#!/bin/sh
# Wait until the Linux serial shell actually answers, and say what it is doing.
#
# Polls for a real response instead of sleeping a fixed interval: the MediaTek
# preloader exposes its own CDC port a few seconds into boot, so the presence of
# /dev/cu.usbmodem* proves nothing. Asking the shell to reply is the only test
# that distinguishes it, and it returns as soon as the device is ready rather
# than after a guessed delay.
cd "$(dirname "$0")/.."
DEADLINE=$(( $(date +%s) + ${1:-90} ))
last=""
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
    if ls /dev/cu.usbmodem* >/dev/null 2>&1; then
        state="port present"
        if timeout 8 python3 tools/sercmd.py 'echo __READY__' 2>/dev/null | grep -q __READY__; then
            echo "shell up after $(( $(date +%s) - DEADLINE + ${1:-90} ))s"
            exit 0
        fi
    else
        state="waiting for usb"
    fi
    [ "$state" != "$last" ] && { echo "  $state"; last="$state"; }
    sleep 1
done
echo "TIMEOUT: no shell after ${1:-90}s"
exit 1
