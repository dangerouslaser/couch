#!/bin/sh
# Run on the remote after the boot-health gate has completed. The test binary
# must live beside couch-gui so assets/config resolve exactly as in production.
# Usage: sh bench-gui.sh couch-gui-perf-baseline 30
set -eu
case "${1:-}" in couch-gui-perf-*) ;; *) echo "expected a couch-gui-perf-* binary" >&2; exit 2;; esac
case "$1" in */*) exit 2;; esac
seconds=${2:-30}
case "$seconds" in ''|*[!0-9]*) exit 2;; esac
[ "$seconds" -ge 5 ] && [ "$seconds" -le 120 ]
[ -x "/opt/couch/$1" ]
[ "$(cut -d. -f1 /proc/uptime)" -gt 160 ] || { echo "wait for boot health validation" >&2; exit 1; }
gui=$(pidof couch-gui)
parent=$(awk '{print $4}' "/proc/$gui/stat")
test_pid=
cleanup() {
    [ -z "$test_pid" ] || kill -TERM "$test_pid" 2>/dev/null || true
    kill -CONT "$parent" 2>/dev/null || true
}
trap cleanup EXIT HUP INT TERM
kill -STOP "$parent"
kill -TERM "$gui"
chroot /proc/1/root /bin/busybox env COUCH_NAV=1 COUCH_SLIDE=1 COUCH_DIM_S=3600 COUCH_OFF_S=0 "/mnt/alpine/opt/couch/$1" > /tmp/gui-benchmark.log 2>&1 &
test_pid=$!
sleep "$seconds"
cat /tmp/gui-benchmark.log
cat "/proc/$test_pid/status" | sed -n '/VmRSS:/p;/voluntary_ctxt_switches:/p'
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq
