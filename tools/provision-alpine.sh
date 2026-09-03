#!/bin/sh
# Build the base Alpine rootfs on the cache partition, from a clean device.
#
# WiFi packages are baked in deliberately: without wpa_supplicant already present
# there is no network, and without a network apk cannot fetch wpa_supplicant. The
# packages are side-loaded offline over adb, which also makes the base image
# reproducible rather than something assembled by hand.
#
# Run from Android. Destroys whatever is on the cache partition.
set -e
cd "$(dirname "$0")/.."
. tools/env.sh

ALPINE_VER=3.21
ALPINE_REL=3.21.7
MIRROR=https://dl-cdn.alpinelinux.org/alpine
ROOTFS=alpine-minirootfs-$ALPINE_REL-armv7.tar.gz
BB=/data/local/tmp/busybox
MNT=/data/local/tmp/alp

# Alpine's own wpa_supplicant, not the vendor one: the vendor build logs solely
# to Android's logd and exits silently when logd is not running, which makes it
# undebuggable here.
PKGS="wpa_supplicant libnl3 dbus-libs pcsc-lite-libs"

mkdir -p build/apks

[ -f "build/$ROOTFS" ] || {
    echo "fetching $ROOTFS ..."
    curl -sL --max-time 300 -o "build/$ROOTFS" "$MIRROR/v$ALPINE_VER/releases/armv7/$ROOTFS"
}

echo "resolving packages ..."
IDX=$(curl -s --max-time 60 "$MIRROR/v$ALPINE_VER/main/armv7/")
for pkg in $PKGS; do
    f=$(echo "$IDX" | grep -oE "${pkg}-[0-9][^\"]*\.apk" | head -1)
    [ -n "$f" ] || { echo "cannot resolve $pkg"; exit 1; }
    [ -f "build/apks/$f" ] || curl -sL --max-time 120 -o "build/apks/$f" \
        "$MIRROR/v$ALPINE_VER/main/armv7/$f"
    printf '  %s\n' "$f"
done

$ADB root >/dev/null 2>&1 || true
sleep 2; $ADB wait-for-device
$ADB shell "[ -x $BB ]" 2>/dev/null || { $ADB push build/busybox-armv7l $BB >/dev/null; $ADB shell "chmod 755 $BB"; }

echo "formatting cache and unpacking rootfs ..."
$ADB shell "umount /cache 2>/dev/null; umount $MNT 2>/dev/null; \
    mke2fs -t ext4 -b 4096 -L alpine -F $PART_CACHE >/dev/null 2>&1; \
    mkdir -p $MNT && mount -t ext4 $PART_CACHE $MNT"
$ADB push "build/$ROOTFS" /data/local/tmp/rootfs.tgz >/dev/null
# busybox tar, not Android's toybox: symlinks and permissions must survive.
$ADB shell "cd $MNT && $BB tar xzf /data/local/tmp/rootfs.tgz"

echo "installing packages offline ..."
$ADB shell "mkdir -p $MNT/opt/apks $MNT/opt/ha100"
for f in build/apks/*.apk; do $ADB push "$f" "$MNT/opt/apks/" >/dev/null; done
$ADB shell "cd $MNT && printf '%s\n' \
    '$MIRROR/v$ALPINE_VER/main' '$MIRROR/v$ALPINE_VER/community' > etc/apk/repositories
    echo 'nameserver 1.1.1.1' > etc/resolv.conf
    echo ha100 > etc/hostname"
# apk runs inside the chroot so it resolves against the new root, not Android.
$ADB shell "$BB chroot $MNT /sbin/apk add --allow-untrusted --no-network /opt/apks/*.apk 2>&1 | tail -3"

echo "installing our payload ..."
$ADB push stage2/stage2.sh   "$MNT/opt/ha100/stage2.sh"   >/dev/null
$ADB push stage2/wifi-conf.sh "$MNT/opt/ha100/wifi-conf.sh" >/dev/null
[ -f build/fbcon ] && $ADB push build/fbcon "$MNT/opt/ha100/fbcon" >/dev/null
$ADB shell "chmod 755 $MNT/opt/ha100/*; \
    $BB chroot $MNT /sbin/apk info 2>/dev/null | tr '\n' ' '; echo; \
    ls $MNT/sbin/wpa_supplicant; sync; umount $MNT && echo 'provisioned, unmounted'"
