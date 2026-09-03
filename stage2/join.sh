#!/bin/busybox sh
# Leave AP mode and join a network, after the portal has saved credentials.
BB=/bin/busybox
A=/mnt/alpine
WANT="$1"

$BB chroot $A /bin/sh -c 'killall hostapd dnsmasq 2>/dev/null'
$BB sleep 1
$BB chroot $A /sbin/ip addr flush dev wlan0 2>/dev/null
$BB chroot $A /sbin/ip link set wlan0 down 2>/dev/null
$BB chroot $A /sbin/iw dev wlan0 set type managed 2>/dev/null
$BB chroot $A /sbin/ip link set wlan0 up 2>/dev/null

$BB sh /opt/couch/wifi-conf.sh "" /tmp/wpa.conf >/dev/null 2>&1
$BB chroot $A /sbin/wpa_supplicant -i wlan0 -Dnl80211 -c /tmp/wpa.conf -B >/tmp/wpa.log 2>&1

n=0
while [ $n -lt 30 ]; do
    ST=$($BB chroot $A /sbin/wpa_cli -i wlan0 status 2>/dev/null | $BB grep "^wpa_state=" | $BB cut -d= -f2)
    [ "$ST" = COMPLETED ] && break
    $BB sleep 1; n=$((n + 1))
done
[ "$ST" = COMPLETED ] && $BB chroot $A /sbin/udhcpc -i wlan0 -n -q -t 10 >/tmp/dhcp.log 2>&1
IP=$($BB ifconfig wlan0 2>/dev/null | $BB sed -n 's/.*inet addr:\([0-9.]*\).*/\1/p')
echo "joined ${WANT:-network} state=$ST ip=${IP:-none}"
