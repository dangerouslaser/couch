#!/bin/sh
# Reboot into Linux and claim the serial session before its 4-minute self-reboot.
set -e
cd "$(dirname "$0")/.."
. tools/env.sh

$ADB reboot recovery
echo "rebooting into Linux..."
sh "$(dirname "$0")/wait-shell.sh" 90 || exit 1
python3 tools/sercmd.py 'touch /tmp/stay; echo CLAIMED'
