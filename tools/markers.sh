#!/bin/sh
# Read the offline progress markers and kernel logs that init writes into the tail
# of the recovery partition. This is the debug channel when USB gives us nothing.
cd "$(dirname "$0")/.."
. tools/env.sh
$ADB root >/dev/null 2>&1 || true
sleep 2; $ADB wait-for-device
echo "=== stage markers ==="
i=0
while [ $i -le 12 ]; do
    printf '  '
    $ADB exec-out "dd if=$PART_EXPDB bs=512 skip=$((MARKER_SECTOR+i)) count=1" 2>/dev/null \
        | tr -d '\0' | head -1
    i=$((i+1))
done
mkdir -p build
$ADB exec-out "dd if=$PART_EXPDB bs=512 skip=$DMESG_SECTOR count=2048" 2>/dev/null \
    | tr -d '\0' > build/linux-dmesg.txt
$ADB exec-out "dd if=$PART_EXPDB bs=512 skip=$DMESG2_SECTOR count=2048" 2>/dev/null \
    | tr -d '\0' > build/linux-dmesg2.txt
echo "kernel logs -> build/linux-dmesg.txt, build/linux-dmesg2.txt"
