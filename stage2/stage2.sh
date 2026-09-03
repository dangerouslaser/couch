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
mark() { $BB echo "[$($BB cut -d. -f1 /proc/uptime)s] $2" | $BB dd of=$LOG bs=512 seek=$1 count=1 conv=notrunc 2>/dev/null; }

echo "= stage2 running from $(dirname "$0")"
[ -e /dev/mmcblk0p13 ] || $BB mknod /dev/mmcblk0p13 b 179 13 2>/dev/null

# MediaTek's connectivity blobs. Couch ships its own copies so it does not
# depend on Android still being installed: mounting Android's /system and
# /vendor works only until someone flashes over them, and then WiFi disappears
# and takes the setup portal with it. Android's partitions are used only as a
# fallback for a device that still has them.
$BB mkdir -p /system /vendor /dev/__properties__ /usr/share/udhcpc
BUNDLE="$(dirname "$0")"
if [ -d "$BUNDLE/vendor/lib/modules" ]; then
    $BB mount -o bind "$BUNDLE/vendor" /vendor 2>/dev/null
    $BB mount -o bind "$BUNDLE/system" /system 2>/dev/null
    VSRC=bundled
else
    $BB mount -t ext4 -o ro /dev/mmcblk0p21 /system 2>/dev/null
    $BB mount -t ext4 -o ro /dev/mmcblk0p14 /vendor 2>/dev/null
    VSRC=android-partitions
fi
echo "= vendor blobs: $VSRC"

# With no devtmpfs only the nodes baked into the cpio exist. wmt_launcher opens
# /dev/ttyMT2 for the chip's STP transport, and without it the chip never
# configures and power-on fails with -3.
for spec in ttyMT1:210 ttyMT2:211 ttyMT3:212; do
    n=${spec%%:*}; m=${spec##*:}
    [ -e /dev/$n ] || $BB mknod /dev/$n c 204 $m 2>/dev/null
done
# Android satisfies these with symlinks; the bundle has to as well.
[ -e /system/vendor ]      || $BB ln -s /vendor /system/vendor 2>/dev/null
[ -e /system/etc/firmware ] || { $BB mkdir -p /system/etc 2>/dev/null
                                 $BB ln -s /vendor/firmware /system/etc/firmware 2>/dev/null; }
echo "= nodes: ttyMT2 $([ -e /dev/ttyMT2 ] && echo ok || echo MISSING), /system/vendor $([ -e /system/vendor ] && echo ok || echo MISSING)"
mark 1 "S1 vendor=$VSRC bundle=$BUNDLE mods=$($BB ls /vendor/lib/modules 2>/dev/null | $BB wc -l)"
$BB mount -t tmpfs tmpfs /dev/__properties__ 2>/dev/null
( cd /dev/__properties__ && $BB tar xzf "$PROPS" 2>/dev/null )
echo "= /system $([ -x /system/bin/linker ] && echo ok || echo FAIL)  /vendor $([ -d /vendor/lib/modules ] && echo ok || echo FAIL)  props $($BB ls /dev/__properties__ | $BB wc -l)"

# Expose the Android tree inside the chroot for interactive debugging.
A=/mnt/alpine
$BB mkdir -p $A/system $A/vendor
$BB mount -o bind /system $A/system 2>/dev/null
$BB mount -o bind /vendor $A/vendor 2>/dev/null
# A bind of /dev does not carry its submounts, so the property area is invisible
# inside the chroot and every Android binary run there sees zero properties.
$BB mkdir -p $A/dev/__properties__
$BB mount -o bind /dev/__properties__ $A/dev/__properties__ 2>/dev/null
echo "= chroot props: $($BB ls $A/dev/__properties__ | $BB wc -l)"

# --- WiFi -------------------------------------------------------------------
#
# Order matters more than anything else here.
#
# wmt_loader reads persist.mtk.wcn.combo.chipid. If that property is already set
# it assumes detection has happened and exits without ever opening /dev/wmtdetect
# - so restoring Android's property area BEFORE this point silently prevents the
# chip from ever being detected. Run the loader with no property area, let it
# read the chip id from hardware (ioctl on /dev/wmtdetect returns 0x6580), then
# mount the properties for everything that comes after.
#
# wmt_drv also registers stpwmt (major 190) only *after* successful detection,
# and there is no devtmpfs, so the node has to be made by hand afterwards.
WIFI=1
if [ "$WIFI" = "1" ]; then
    $BB umount /dev/__properties__ 2>/dev/null
    $BB umount $A/dev/__properties__ 2>/dev/null

    # userdata now holds our rootfs, so there is no Android /data to borrow.
    # The wlan driver logs an NVRAM warning without it and works regardless.

    $BB insmod /vendor/lib/modules/wmt_drv.ko 2>/dev/null
    $BB sleep 1; $BB mdev -s
    LD_LIBRARY_PATH=/system/lib:/vendor/lib /vendor/bin/wmt_loader >/tmp/wmt.log 2>&1
    echo "= wmt_loader rc=$? chip=$($BB grep -oE '190 mtk_stp_wmt' /proc/devices || echo NOT-DETECTED)"

    # stpwmt exists only once detection succeeded; no devtmpfs, so mknod it.
    [ -e /dev/stpwmt ] || $BB mknod /dev/stpwmt c 190 0 2>/dev/null

    $BB insmod /vendor/lib/modules/wmt_chrdev_wifi.ko 2>/dev/null
    $BB insmod /vendor/lib/modules/wlan_drv.ko 2>/dev/null
    $BB mdev -s
    [ -e /dev/wmtWifi ] || $BB mknod /dev/wmtWifi c 153 0 2>/dev/null

    # No property area is restored. The snapshot was device-specific (it carries
    # this unit's serial number among other things) and cannot ship in a
    # distributable image; it also broke detection above. Restored here only if
    # a snapshot happens to be present, which it is not by default.
    PROPS=""
    [ -f "$BUNDLE/props.tar.gz" ] && PROPS="$BUNDLE/props.tar.gz"
    [ -z "$PROPS" ] && [ -f /extra/props.tar.gz ] && PROPS=/extra/props.tar.gz
    if [ -n "$PROPS" ]; then
        $BB mount -t tmpfs tmpfs /dev/__properties__ 2>/dev/null
        ( cd /dev/__properties__ && $BB tar xzf "$PROPS" 2>/dev/null )
        $BB mkdir -p $A/dev/__properties__
        $BB mount -o bind /dev/__properties__ $A/dev/__properties__ 2>/dev/null
    fi

    LD_LIBRARY_PATH=/system/lib:/vendor/lib:/system/lib/hw:/vendor/lib/hw \
        /vendor/bin/wmt_launcher -p /vendor/firmware/ >/tmp/wl.log 2>&1 &
    # Wait for the launcher to actually configure the chip rather than guessing.
    # It reports "set STP mode success" once the transport is up; powering on
    # WiFi before that gives pwr_on fail(-3).
    n=0
    while [ $n -lt 20 ]; do
        $BB dmesg | $BB grep -q "set STP mode success" && break
        $BB sleep 1; n=$((n+1))
    done
    echo "= launcher $($BB pidof wmt_launcher >/dev/null && echo running || echo dead), stp after ${n}s"
    mark 5 "S5 launcher=$($BB pidof wmt_launcher >/dev/null && echo up || echo dead) stp=${n}s"
    $BB echo 1 > /dev/wmtWifi 2>/tmp/wifion.err
    $BB sleep 3; $BB mdev -s
    $BB ifconfig wlan0 up 2>/dev/null
    echo "= wlan0 $([ -d /sys/class/net/wlan0 ] && echo UP || echo MISSING)  $($BB cat /tmp/wifion.err 2>/dev/null)"

    # Credentials come from Android's own store, read on the device at runtime,
    # so they never live in this repo or the boot image. userdata is f2fs.
    # Android's store is a bonus, not a requirement. After installing to
    # userdata it is gone, and gating on it skipped our own saved networks too -
    # sending a perfectly configured device to the setup portal. wifi-conf.sh
    # merges /opt/couch/networks.conf with the store and copes if either is
    # missing, so the only real precondition is having a radio.
    CFG=/mnt/data/misc/wifi/WifiConfigStore.xml
    if [ -d /sys/class/net/wlan0 ]; then
        # Every stored network, not just the first: the first entry is not
        # necessarily the one in range.
        $BB sh "$(dirname "$0")/wifi-conf.sh" "$CFG" /tmp/wpa.conf | $BB tail -1
        NETS=$($BB grep -c "^network=" /tmp/wpa.conf 2>/dev/null)
        echo "= known networks: ${NETS:-0}"

        [ "${NETS:-0}" -gt 0 ] && $BB chroot $A /sbin/wpa_supplicant -i wlan0 \
            -Dnl80211 -c /tmp/wpa.conf -B >/tmp/wpa.log 2>&1

        # Wait for association rather than guessing: a fixed sleep runs dhcp
        # while still SCANNING and it fails for no visible reason. With nothing
        # to associate to there is nothing to wait for - go straight to setup.
        n=0
        while [ "${NETS:-0}" -gt 0 ] && [ $n -lt 30 ]; do
            ST=$($BB chroot $A /sbin/wpa_cli -p /tmp/wpa -i wlan0 status 2>/dev/null | $BB grep "^wpa_state=" | $BB cut -d= -f2)
            [ "$ST" = "COMPLETED" ] && break
            $BB sleep 1; n=$((n+1))
        done
        NET=$($BB chroot $A /sbin/wpa_cli -p /tmp/wpa -i wlan0 status 2>/dev/null | $BB grep "^ssid=" | $BB cut -d= -f2)
        echo "= assoc ${ST:-none} ${NET:+to \"$NET\"} after ${n}s"

        # Run dhcp inside the chroot: busybox udhcpc does nothing without
        # /usr/share/udhcpc/default.script, and Alpine ships a maintained one.
        # Run from the initramfs it takes the lease and never applies it, which
        # looks exactly like dhcp failing.
        if [ "$ST" = "COMPLETED" ]; then
            $BB chroot $A /sbin/udhcpc -i wlan0 -n -q -t 10 >/tmp/dhcp.log 2>&1
            IP=$($BB ifconfig wlan0 2>/dev/null | $BB sed -n 's/.*inet addr:\([0-9.]*\).*/\1/p')
            if [ -n "$IP" ]; then
                $BB cp /etc/resolv.conf $A/etc/resolv.conf 2>/dev/null
                # Root telnet, no password, on the LAN. Fine on a bench, not
                # something to ship - sshd with an enrolled key is the shipped
                # remote path, so this stays behind the debug flag.
                [ -n "$COUCH_DEBUG" ] && $BB telnetd -l /bin/sh -p 23 2>/dev/null
                echo "= NETWORK UP  $IP"
            else
                echo "= dhcp FAILED $($BB tail -1 /tmp/dhcp.log 2>/dev/null)"
            fi
        fi
        mark 6 "S6 assoc=$ST ip=${IP:-none}"
    fi

    # No network: bring up the setup portal so WiFi can be configured without a
    # USB shell. The passphrase is shown on the panel, so setup requires being
    # able to see the device.
    if [ -z "$IP" ]; then
        $BB sh "$(dirname "$0")/confirm.sh" >/tmp/confirm.log 2>&1 &
        $BB sh "$(dirname "$0")/portal.sh"
    else
        # sshd only once we are actually on a network, and only if the user
        # enrolled a key or set a root password through the setup portal. The
        # shipped image trusts nobody by default.
        echo "= $($BB chroot $A /bin/sh /opt/couch/sshd.sh) ${IP:+on $IP:22}"
    fi
else
    echo "= wifi parked"
fi

# --- the UI ------------------------------------------------------------------
# Start it last, so anything above still reports to the screen through fbcon.
# couch-gui takes the panel over when it starts and shows its own splash.
# Only when we have a network. With no known WiFi the setup portal owns the
# screen: it prints its SSID and one-time passphrase through fbcon, and the GUI
# would paint straight over them.
GUI="$(dirname "$0")/couch-gui"
# With no network, portal.sh has already left /tmp/couch.setup behind and the
# GUI reads it at startup, showing the join QR instead of the room UI. Nothing
# to pass here: a variable set in this loop's environment could never be
# cleared again without killing the loop.
[ -f /tmp/couch.setup ] && echo "= couch-gui starting in setup mode"
if [ -x "$GUI" ]; then
    ( while true; do
        "$GUI" >/tmp/gui.log 2>&1
        echo "= couch-gui exited ($?), restarting" >> /tmp/gui.log
        $BB sleep 2
      done ) &
    echo "= couch-gui started"
else
    echo "= no couch-gui at $GUI"
fi

$BB dmesg | $BB dd of=$LOG bs=512 seek=2048 conv=notrunc 2>/dev/null
mark $((BASE+4)) "S4 stage2 done"
echo ""
echo "= READY  uptime $($BB cut -d. -f1 /proc/uptime)s"
echo "= edit:  tools/push.py stage2/stage2.sh /mnt/alpine/opt/couch/stage2.sh"
echo "= rerun: tools/relinux.sh"

echo "= fast-loop test marker: 22:12:42"
