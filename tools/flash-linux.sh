#!/bin/sh
# Flash a boot image to the slot Couch boots from (p8), from a running Couch
# or its recovery, over ssh. This is the flash path since the slots were
# swapped: there is no Android on the device to run adb against.
#
#   tools/flash-linux.sh build/couch-test.img         # then reboot -f on the device
#
# The image goes over scp, is checksummed, written with dd to the block device,
# read back and compared, and only then is the BCB cleared so the next boot is
# p8. Every step refuses to continue on a mismatch. The block-device check
# matters: under Couch the nodes are /dev/mmcblk0pN, and writing to a path that
# does not exist creates a regular file in RAM and "verifies" perfectly.
set -e
cd "$(dirname "$0")/.."
IMG=${1:?usage: tools/flash-linux.sh IMAGE}
IP=${COUCH_IP:?set COUCH_IP to the device address}
KEY=${COUCH_KEY:-$HOME/.ssh/couch_dev}
O="-i $KEY -o IdentitiesOnly=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=10 -o BatchMode=yes"

[ "$(head -c 8 "$IMG")" = "ANDROID!" ] || { echo "REFUSING: $IMG is not a boot image"; exit 1; }
WANT=$(md5 -q "$IMG" 2>/dev/null || md5sum "$IMG" | cut -c1-32)
SZ=$(wc -c < "$IMG" | tr -d ' ')

scp $O "$IMG" root@$IP:/tmp/flash.img
ssh $O root@$IP "sh -s $SZ $WANT" <<'REMOTE'
set -e
SZ=$1; WANT=$2
IMG=/tmp/flash.img; P8=/dev/mmcblk0p8; BCB=/dev/mmcblk0p10
for n in $P8 $BCB; do [ -b "$n" ] || { echo "REFUSING: $n is not a block device"; exit 1; }; done
[ "$(md5sum $IMG | cut -c1-32)" = "$WANT" ] || { echo "REFUSING: transfer corrupted"; exit 1; }
dd if=$IMG of=$P8 bs=1048576 conv=notrunc 2>/dev/null; sync
BLK=$(( (SZ + 65535) / 65536 ))
RB=$(dd if=$P8 bs=65536 count=$BLK 2>/dev/null | head -c $SZ | md5sum | cut -c1-32)
[ "$RB" = "$WANT" ] || { echo "MISMATCH after write - not clearing the BCB"; exit 1; }
echo "VERIFIED: p8 matches $IMG"
dd if=/dev/zero of=$BCB bs=512 count=1 conv=notrunc 2>/dev/null; sync
echo "BCB cleared: the next boot is p8. reboot -f when ready."
REMOTE
