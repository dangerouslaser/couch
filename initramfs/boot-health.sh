#!/bin/sh
# Read-only local health check. Optional root supports offline snapshot tests.
set -eu
root=${1:-/}
read -r health_pid health_stamp extra < "$root/tmp/couch-gui.health"
[ -z "$extra" ]
case "$health_pid" in ''|*[!0-9]*) exit 1;; esac
case "$health_stamp" in ''|*[!0-9]*) exit 1;; esac
[ "$health_pid" -gt 1 ]
[ "$(cat "$root/proc/$health_pid/comm")" = couch-gui ]
now=$(cut -d. -f1 < "$root/proc/uptime")
case "$now" in ''|*[!0-9]*) exit 1;; esac
[ "$health_stamp" -le "$now" ]
[ "$((now - health_stamp))" -le 10 ]
printf '%s %s\n' "$health_pid" "$health_stamp"
