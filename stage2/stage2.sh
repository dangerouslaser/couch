#!/bin/busybox sh
# Second stage, kept on the Alpine partition rather than in the initramfs so it
# can be changed with tools/push.py and re-run with tools/relinux.sh - no adb,
# no reflash, no trip through Android.
#
# The initramfs has already mounted /proc, /sys, /tmp and Alpine, opened the USB
# serial shell and started fbcon, so a broken edit here still leaves a shell.

BB=/bin/busybox
LOG=/dev/mmcblk0p13      # expdb: scratch for markers and logs
BASE=0
mark() { $BB echo "$2" | $BB dd of=$LOG bs=512 seek=$1 count=1 conv=notrunc 2>/dev/null; }

echo "= stage2 running from $(dirname "$0")"

$BB mkdir -p /system /vendor /dev/__properties__
$BB mount -t ext4 -o ro /dev/mmcblk0p21 /system 2>/dev/null
$BB mount -t ext4 -o ro /dev/mmcblk0p14 /vendor 2>/dev/null
$BB mount -t tmpfs tmpfs /dev/__properties__ 2>/dev/null
( cd /dev/__properties__ && $BB tar xzf /extra/props.tar.gz 2>/dev/null )
echo "= /system $([ -x /system/bin/linker ] && echo ok || echo FAIL)  /vendor $([ -d /vendor/lib/modules ] && echo ok || echo FAIL)  props $($BB ls /dev/__properties__ | $BB wc -l)"

# WiFi stays parked: writing /dev/wmtWifi with the chip half-initialised panics
# inside wmt_drv and hard-resets the device. Set WIFI=1 to retry; it is guarded.
WIFI=0
if [ "$WIFI" = "1" ]; then
    $BB insmod /vendor/lib/modules/wmt_drv.ko 2>/dev/null
    n=0; while [ $n -lt 15 ]; do $BB mdev -s; [ -e /dev/stpwmt ] && break; $BB sleep 1; n=$((n+1)); done
    echo "= stpwmt $([ -e /dev/stpwmt ] && echo "ok after ${n}s" || echo MISSING)"
    LD_LIBRARY_PATH=/system/lib:/vendor/lib /vendor/bin/wmt_loader >/tmp/wmt.log 2>&1; RC=$?
    $BB insmod /vendor/lib/modules/wmt_chrdev_wifi.ko 2>/dev/null
    $BB insmod /vendor/lib/modules/wlan_drv.ko 2>/dev/null
    $BB mdev -s
    LD_LIBRARY_PATH=/system/lib:/vendor/lib /vendor/bin/wmt_launcher -p /vendor/firmware/ >/dev/null 2>&1 &
    $BB sleep 8
    OK=yes
    $BB dmesg | $BB grep -q "pwr_on fail"      && OK=no
    $BB dmesg | $BB grep -q "p_ic_ops is NULL" && OK=no
    [ "$RC" = 0 ] || OK=no
    echo "= wmt rc=$RC chip_ready=$OK"
    [ "$OK" = yes ] && $BB echo 1 > /dev/wmtWifi 2>/dev/null
    $BB sleep 3; $BB mdev -s
    echo "= net: $($BB ls /sys/class/net/ | $BB tr '\n' ' ')"
else
    echo "= wifi parked"
fi

$BB dmesg | $BB dd of=$LOG bs=512 seek=2048 conv=notrunc 2>/dev/null
mark $((BASE+4)) "S4 stage2 done"
echo ""
echo "= READY  uptime $($BB cut -d. -f1 /proc/uptime)s"
echo "= edit:  tools/push.py stage2/stage2.sh /mnt/alpine/opt/ha100/stage2.sh"
echo "= rerun: tools/relinux.sh"

echo "= fast-loop test marker: 22:12:42"
