#!/bin/busybox sh
# Setup hotspot + captive portal, for when no known network is reachable and
# there is nobody on the USB serial line to fix it.
#
# phy0 reports AP mode support ("iw list"), so the same radio that failed to
# associate can host the setup network. Runs entirely inside the Alpine chroot
# because that is where hostapd, dnsmasq and the busybox httpd live.
BB=/bin/busybox
A=/mnt/alpine
AP_IF=wlan0
AP_IP=192.168.4.1
AP_SSID=${AP_SSID:-Sanytron-Setup}

echo "= starting setup portal on $AP_SSID ($AP_IP)"

# Scan BEFORE switching to AP mode: a radio in AP mode cannot scan, so the list
# of networks to offer has to be captured now and cached for the portal to serve.
$BB chroot $A /bin/sh -c '
    pidof wpa_supplicant >/dev/null || wpa_supplicant -i wlan0 -Dnl80211 -c /dev/null -B >/dev/null 2>&1
    wpa_cli -i wlan0 scan >/dev/null 2>&1
    sleep 5
    wpa_cli -i wlan0 scan_results 2>/dev/null
' > /tmp/scan.raw 2>/dev/null

# bssid / freq / signal / flags / ssid -> json, strongest first, deduplicated.
$BB awk -F"\t" '
    NF >= 5 && $5 != "" && $5 !~ /^\\x00/ {
        if (!($5 in best) || $3 > best[$5]) { best[$5] = $3; sec[$5] = ($4 ~ /WPA|WEP|PSK|SAE/) ? "true" : "false" }
    }
    END {
        printf "["
        n = 0
        for (s in best) { if (n++) printf ","
            printf "{\"ssid\":\"%s\",\"signal\":%d,\"secure\":%s}", s, best[s], sec[s] }
        printf "]"
    }
' /tmp/scan.raw > /tmp/scan.json 2>/dev/null
echo "= cached $($BB grep -o "ssid" /tmp/scan.json | $BB wc -l) networks for the portal"

# The radio cannot be a station and an AP at once.
$BB chroot $A /bin/sh -c 'killall wpa_supplicant 2>/dev/null; killall udhcpc 2>/dev/null'
$BB sleep 1

# A random WPA2 passphrase per boot, shown on the device's own screen. Joining
# the setup network therefore requires physically looking at the remote, which
# is what makes it safe to enrol an SSH key through this portal at all. An open
# AP would let anyone in radio range take permanent root.
PASS=$($BB dd if=/dev/urandom bs=1 count=16 2>/dev/null | $BB md5sum | $BB cut -c1-10)
echo "$PASS" > /tmp/portal.pass

cat > $A/tmp/hostapd.conf <<CONF
interface=$AP_IF
driver=nl80211
ssid=$AP_SSID
hw_mode=g
channel=6
auth_algs=1
wmm_enabled=0
ignore_broadcast_ssid=0
wpa=2
wpa_passphrase=$PASS
wpa_key_mgmt=WPA-PSK
rsn_pairwise=CCMP
CONF
$BB chmod 600 $A/tmp/hostapd.conf

# busybox httpd serves the portal for any path via the 404 handler, so whatever
# URL the connectivity check asks for lands on the setup page.
$BB mkdir -p $A/opt/couch/www
printf 'E404:/index.html\nI:index.html\n' > $A/opt/couch/www/httpd.conf

$BB chroot $A /sbin/ip link set $AP_IF down 2>/dev/null
$BB chroot $A /sbin/iw dev $AP_IF set type __ap 2>/dev/null
$BB chroot $A /sbin/ip link set $AP_IF up 2>/dev/null
$BB chroot $A /sbin/ip addr flush dev $AP_IF 2>/dev/null
$BB chroot $A /sbin/ip addr add $AP_IP/24 dev $AP_IF 2>/dev/null

$BB chroot $A /usr/sbin/hostapd -B /tmp/hostapd.conf >/tmp/hostapd.log 2>&1
$BB sleep 2
$BB chroot $A /usr/sbin/dnsmasq -C /tmp/dnsmasq.conf >/tmp/dnsmasq2.log 2>&1
$BB chroot $A /bin/busybox httpd -p 80 -h /opt/couch/www -c /opt/couch/www/httpd.conf

RUNNING=$($BB chroot $A /bin/sh -c 'pidof hostapd >/dev/null && echo yes || echo no')
echo "= hostapd=$RUNNING  dnsmasq=$($BB chroot $A /bin/sh -c 'pidof dnsmasq >/dev/null && echo yes || echo no')  httpd=$($BB chroot $A /bin/sh -c 'pidof busybox >/dev/null && echo yes || echo no')"
# Put the credentials on the panel: this is the only place the passphrase is
# shown, and it is what ties setup to physical possession of the device.
{
    echo ""
    echo "=================================="
    echo "  WI-FI SETUP"
    echo "  network:  $AP_SSID"
    echo "  password: $PASS"
    echo "  then open:  http://$AP_IP/"
    echo "=================================="
    echo ""
} > /tmp/portal.banner
$BB cat /tmp/portal.banner

# Stop after a while rather than leaving a setup network up for ever.
( $BB sleep ${PORTAL_TIMEOUT:-600}
  $BB chroot $A /bin/sh -c 'killall hostapd dnsmasq 2>/dev/null'
  echo "= setup portal timed out" ) &
