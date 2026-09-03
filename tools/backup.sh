#!/bin/sh
# Dump every partition to $BACKUP_DIR before touching anything.
#
# nvram, nvdata, protect1 and protect2 hold this unit's WiFi/BT calibration.
# They are not recoverable from any firmware image if lost. Run this first.
set -e
cd "$(dirname "$0")/.."
. tools/env.sh
BB=/data/local/tmp/busybox
mkdir -p "$BACKUP_DIR"

$ADB root >/dev/null 2>&1 || true
sleep 2; $ADB wait-for-device
$ADB shell "[ -x $BB ]" 2>/dev/null || {
    curl -sL -o /tmp/bb https://busybox.net/downloads/binaries/1.31.0-defconfig-multiarch-musl/busybox-armv7l
    $ADB push /tmp/bb $BB >/dev/null; $ADB shell "chmod 755 $BB"
}

# name:partition:size_in_kb, from /proc/partitions and the by-name symlinks.
for spec in \
  proinfo:p1:3072 nvram:p2:5120 protect1:p3:10240 protect2:p4:10240 \
  seccfg:p5:256 lk:p6:384 lk2:p7:384 boot:p8:16384 recovery:p9:16384 \
  para:p10:512 logo:p11:8192 odmdtbo:p12:16384 expdb:p13:10240 \
  vendor:p14:294912 frp:p15:1024 nvdata:p16:32768 metadata:p17:39936 \
  oemkeystore:p18:2048 secro:p19:6144 keystore:p20:8192 system:p21:1253376 \
  flashinfo:p24:16384 preloader_a:boot0:4096 preloader_b:boot1:4096
do
    name=${spec%%:*}; rest=${spec#*:}; part=${rest%%:*}; kb=${rest##*:}
    out="$BACKUP_DIR/$name.img"
    # dd's stats go to stdout through exec-out, so trim to the exact size after.
    $ADB exec-out "dd if=/dev/block/mmcblk0$part bs=1048576" 2>/dev/null > "$out"
    python3 -c "import os,sys; f=sys.argv[1]; n=int(sys.argv[2])*1024
os.truncate(f,n) if os.path.getsize(f)!=n else None" "$out" "$kb"
    printf '%-14s %12s bytes\n' "$name" "$(stat -f%z "$out" 2>/dev/null || stat -c%s "$out")"
done
echo "backup complete in $BACKUP_DIR"
