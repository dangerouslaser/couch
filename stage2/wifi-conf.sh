#!/bin/busybox sh
# Build a wpa_supplicant config from Android's own saved networks.
#
# Emits every stored network, not just the first: the first entry in the store is
# not necessarily the one in range, and wpa_supplicant will pick whichever it can
# actually see. Credentials are read on the device at runtime and never leave it.
CFG=${1:-/mnt/data/misc/wifi/WifiConfigStore.xml}
OUT=${2:-/tmp/wpa.conf}
[ -f "$CFG" ] || { echo "no wifi store at $CFG" >&2; exit 1; }

{
    echo "ctrl_interface=/tmp/wpa"
    echo "update_config=1"
    awk '
    function flush() {
        if (have) {
            printf "network={\n  ssid=\"%s\"\n", ssid
            if (psk != "") printf "  psk=\"%s\"\n", psk
            else print "  key_mgmt=NONE"
            print "}"
        }
    }
    /<string name="SSID">/ {
        flush()
        s = $0
        sub(/.*<string name="SSID">&quot;/, "", s)
        sub(/&quot;<\/string>.*/, "", s)
        ssid = s; psk = ""; have = 1
    }
    /<string name="PreSharedKey">/ {
        s = $0
        sub(/.*<string name="PreSharedKey">&quot;/, "", s)
        sub(/&quot;<\/string>.*/, "", s)
        psk = s
    }
    END { flush() }
    ' "$CFG"
} > "$OUT"
chmod 600 "$OUT"
echo "networks: $(grep -c '^network=' "$OUT")"
grep '  ssid=' "$OUT"
