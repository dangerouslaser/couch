#!/bin/busybox sh
# Leave setup mode and join the network the portal just saved - in place, with
# no reboot.
#
# There is no radio hand-back to negotiate: the MediaTek driver puts its AP
# personality on a second wiphy (phy1, netdev ap0) and leaves wlan0 on phy0
# untouched the whole time the setup network is up. That is the same property
# that lets the portal rescan while it is serving. So joining is just: stop the
# AP daemons, point a supplicant at the freshly saved credentials, take a lease.
#
# Runs inside the Alpine chroot, called from the save CGI - every daemon the
# portal started lives in there too.
#
# If association fails the remote reboots, which brings the setup portal back
# for another attempt rather than stranding it with neither a network nor an AP.
BB=/bin/busybox
WANT="$1"

echo "joining ${WANT:-network}"

$BB killall hostapd dnsmasq 2>/dev/null
$BB sleep 1
/sbin/ip addr flush dev ap0 2>/dev/null
/sbin/ip link set ap0 down 2>/dev/null

# Rebuild the supplicant config so it picks up what save.sh just wrote, and
# replace the scan-only supplicant portal.sh left running. Its stale control
# socket would block the new one from binding.
$BB sh /opt/couch/wifi-conf.sh "" /tmp/wpa.conf >/dev/null 2>&1
$BB killall wpa_supplicant 2>/dev/null
$BB sleep 1
$BB rm -f /tmp/wpa/wlan0 2>/dev/null

/sbin/ip link set wlan0 up 2>/dev/null
/sbin/wpa_supplicant -i wlan0 -Dnl80211 -c /tmp/wpa.conf -B >/tmp/wpa.log 2>&1

n=0
while [ $n -lt 30 ]; do
    ST=$(/sbin/wpa_cli -p /tmp/wpa -i wlan0 status 2>/dev/null |
         $BB grep "^wpa_state=" | $BB cut -d= -f2)
    [ "$ST" = COMPLETED ] && break
    $BB sleep 1; n=$((n + 1))
done

if [ "$ST" = COMPLETED ]; then
    /sbin/udhcpc -i wlan0 -n -q -t 10 >/tmp/dhcp.log 2>&1
    IP=$(/sbin/ip -4 -o addr show wlan0 2>/dev/null |
         $BB tr -s " " | $BB cut -d" " -f4 | $BB cut -d/ -f1)
fi

if [ -n "$IP" ]; then
    echo "joined ${WANT:-network} as $IP after ${n}s"
    $BB sh /opt/couch/sshd.sh 2>/dev/null
    # Drop the marker and restart the GUI: stage2's supervisor brings it back
    # within seconds, and it reads the marker at startup, so it returns on the
    # room UI instead of the setup screen.
    $BB rm -f /tmp/couch.setup
    $BB kill $($BB pidof couch-gui) 2>/dev/null
else
    echo "join failed (state=${ST:-none}) - rebooting back into setup"
    $BB sync
    $BB reboot -f
fi
