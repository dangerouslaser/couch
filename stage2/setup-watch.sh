#!/bin/busybox sh
# Privileged handoff from local UI to the optional recovery portal. Requests
# are local /tmp files, never HTTP endpoints. Run in the outer boot root.
BB=/bin/busybox
BASE=$(dirname "$0")
A=/mnt/alpine
# A second supervisor must not race a radio mode switch.
$BB mkdir /tmp/couch-setup-watch.lock 2>/dev/null || exit 0
trap 'rmdir /tmp/couch-setup-watch.lock' EXIT
while true; do
    if [ -f /tmp/couch.ap-request ]; then
        $BB rm -f /tmp/couch.ap-request
        if [ ! -f /tmp/couch.setup ]; then
            $BB sh "$BASE/confirm.sh" >/tmp/confirm.log 2>&1 &
            $BB sh "$BASE/portal.sh" >/tmp/portal.log 2>&1
        fi
    fi
    if [ -f /tmp/couch.network-ready ]; then
        $BB rm -f /tmp/couch.network-ready
        # Wi-Fi Save has already tested association and DHCP. SSH still
        # requires prior enrollment; connecting never enables password access.
        $BB chroot "$A" /bin/sh /opt/couch/sshd.sh
    fi
    $BB sleep 1
done
