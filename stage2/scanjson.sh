#!/bin/sh
# wpa_cli "scan_results" (tab separated) on stdin -> JSON on stdout: strongest
# signal first, one entry per SSID. Shared by portal.sh, which caches a scan
# taken just before the radio switches to AP mode, and cgi-bin/scan, which can
# repeat it live. /bin/busybox is the one awk that exists in both the initramfs
# and the Alpine chroot.
exec /bin/busybox awk -F'\t' '
    $1 == "bssid" { next }                      # the header row is not a network
    $1 == "00:00:00:00:00:00" { next }          # the driver injects a bogus row
                                                # here whose SSID is an NVRAM
                                                # error string, not a network
    NF >= 5 && $5 != "" && $5 !~ /^\\x00/ {
        s = $5
        gsub(/\\/, "\\\\", s); gsub(/"/, "\\\"", s)   # keep the JSON parseable
        if (!(s in best) || $3 > best[s]) {
            best[s] = $3
            sec[s] = ($4 ~ /WPA|WEP|PSK|SAE/) ? "true" : "false"
        }
    }
    END {
        printf "["
        n = 0
        for (s in best) { if (n++) printf ","
            printf "{\"ssid\":\"%s\",\"signal\":%d,\"secure\":%s}", s, best[s], sec[s] }
        printf "]"
    }
'
