#!/bin/sh
# Pull the MediaTek connectivity blobs out of the backed-up Android images.
#
# Couch has to carry these itself. Mounting Android's /system and /vendor at
# runtime works only while Android is still installed, which makes the whole
# thing a guest on Android rather than a distribution: flash over the device and
# WiFi disappears, taking the setup portal with it.
#
# Reads the backup images directly with debugfs in a container, so it needs no
# device, no adb, and no loopback mounts (which macOS cannot do for ext4).
set -e
cd "$(dirname "$0")/.."
. tools/env.sh
OUT=build/vendor-bundle
rm -rf $OUT; mkdir -p $OUT/vendor/lib/modules $OUT/vendor/firmware $OUT/vendor/bin \
                      $OUT/system/lib $OUT/system/bin

[ -f "$BACKUP_DIR/vendor.img" ] || { echo "need $BACKUP_DIR/vendor.img (tools/backup.sh)"; exit 1; }
[ -f "$BACKUP_DIR/system.img" ] || { echo "need $BACKUP_DIR/system.img (tools/backup.sh)"; exit 1; }

VENDOR_FILES="lib/modules/wmt_drv.ko lib/modules/wmt_chrdev_wifi.ko lib/modules/wlan_drv.ko \
              bin/wmt_loader bin/wmt_launcher"
# The whole firmware directory, not a hand-picked subset. It is only ~6MB, and
# picking files by name left the chip failing to power up ("pwr_on fail(-3)")
# because the patch set the launcher downloads is not obvious from filenames.
# etc/selinux carries nonplat_property_contexts. bionic's property system needs
# it to map property names to SELinux contexts; without it wmt_launcher spins in
# a 300ms poll loop and never issues the STP-mode ioctl, so stp_init reports
# "no hif info!" and chip power-on fails with -3.
VENDOR_DIRS="firmware etc/selinux"
# The MTK binaries are bionic, so they need Android's linker and libc, at their
# real paths - PT_INTERP is baked in as /system/bin/linker.
# Same reason as vendor/etc/selinux: plat_property_contexts is required for
# property access to work at all. ld.config.txt keeps the linker quiet.
SYSTEM_DIRS="etc/selinux"
SYSTEM_FILES="etc/ld.config.txt xbin/strace bin/linker lib/libc.so lib/libm.so lib/libdl.so lib/libc++.so \
              lib/liblog.so lib/libcutils.so lib/libbacktrace.so lib/libunwind.so \
              lib/libutils.so lib/libnetd_client.so lib/libbase.so lib/liblzma.so"

# Two sources, same result. Docker + the backup images needs no device at all,
# which is what a rebuild on another machine wants. Pulling from a live device
# over adb is the fallback. Either way this is a BUILD-time dependency: the
# resulting image carries the blobs and needs neither Android nor adb to run.
if docker info >/dev/null 2>&1; then
    echo "extracting from the backup images (docker + debugfs) ..."
    docker run --rm -v "$PWD:/w" -v "$BACKUP_DIR:/img:ro" alpine:3.21 sh -c '
        apk add --no-cache e2fsprogs-extra >/dev/null 2>&1
        ok=0
        for f in '"$VENDOR_FILES"'; do
            debugfs -R "dump /$f /w/'"$OUT"'/vendor/$f" /img/vendor.img >/dev/null 2>&1 \
              && [ -s "/w/'"$OUT"'/vendor/$f" ] && ok=$((ok+1)) || echo "  missing vendor/$f"
        done
        for d in '"$VENDOR_DIRS"'; do
            debugfs -R "rdump /$d /w/'"$OUT"'/vendor" /img/vendor.img >/dev/null 2>&1 \
              && ok=$((ok+1)) || echo "  missing vendor/$d/"
        done
        for f in '"$SYSTEM_FILES"'; do
            debugfs -R "dump /$f /w/'"$OUT"'/system/$f" /img/system.img >/dev/null 2>&1 \
              && [ -s "/w/'"$OUT"'/system/$f" ] && ok=$((ok+1)) || echo "  optional system/$f absent"
        done
        echo "extracted $ok files"
    '
elif $ADB shell true >/dev/null 2>&1; then
    echo "docker not running; pulling from the attached device over adb ..."
    sh "$(dirname "$0")/wait-adb.sh" 60 || exit 1
    ok=0
    for f in $VENDOR_FILES; do
        if $ADB pull "/vendor/$f" "$OUT/vendor/$f" >/dev/null 2>&1; then ok=$((ok+1));
        else echo "  missing vendor/$f"; fi
    done
    for d in $SYSTEM_DIRS; do
        $ADB shell "mkdir -p /dev/null" >/dev/null 2>&1
        mkdir -p "$OUT/system/$(dirname "$d")"
        $ADB pull "/system/$d" "$OUT/system/$d" >/dev/null 2>&1 \
            && ok=$((ok+1)) || echo "  missing system/$d/"
    done
    for d in $VENDOR_DIRS; do
        mkdir -p "$OUT/vendor/$(dirname "$d")"
        $ADB pull "/vendor/$d" "$OUT/vendor/$d" >/dev/null 2>&1 \
            && ok=$((ok + $(ls "$OUT/vendor/$d" | wc -l | tr -d " "))) \
            || echo "  missing vendor/$d/"
    done
    for f in $SYSTEM_FILES; do
        $ADB pull "/system/$f" "$OUT/system/$f" >/dev/null 2>&1 && ok=$((ok+1)) \
            || echo "  optional system/$f absent"
    done
    echo "extracted $ok files"
else
    echo "need either a running docker (to read $BACKUP_DIR images) or an attached device"
    exit 1
fi

chmod -R 755 $OUT
echo
du -sh $OUT
find $OUT -type f | sed "s|$OUT/|  |" | sort
