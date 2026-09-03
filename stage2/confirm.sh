#!/bin/busybox sh
# Watch for SSH access requests from the portal - an enrolled key or a root
# password - and require a physical button press to approve them.
#
# A request alone must never be enough to grant root: this proves someone is
# holding the remote. Any key on the keypad counts; reading 16 bytes from an
# evdev node blocks until an input event arrives (struct input_event is 16
# bytes on 32-bit ARM).
#
# Every node is read at once rather than in turn. Read sequentially, a press on
# the second keypad would sit behind a 25s wait on the first, by which point the
# CGI asking has nearly given up.
#
# The result is published for couch-gui, which draws the prompt: this script's
# own output goes to a log, and in setup mode the GUI owns the panel.
BB=/bin/busybox

wait_for_press() {
    PIDS=""
    for ev in /dev/input/event0 /dev/input/event1 /dev/input/event2; do
        [ -e "$ev" ] || continue
        ( $BB dd if="$ev" bs=16 count=1 >/dev/null 2>&1 && : > /tmp/press.ok ) &
        PIDS="$PIDS $!"
    done
    [ -n "$PIDS" ] || return 1

    n=0
    while [ $n -lt 25 ] && [ ! -f /tmp/press.ok ]; do
        $BB sleep 1; n=$((n + 1))
    done
    $BB kill $PIDS 2>/dev/null
    [ -f /tmp/press.ok ]
}

while true; do
    if [ -f /tmp/press.request ]; then
        echo ""
        echo "  >>> PRESS ANY BUTTON ON THE REMOTE TO AUTHORISE SSH ACCESS <<<"
        $BB rm -f /tmp/press.result
        if wait_for_press; then
            echo ok > /tmp/press.result
            echo "  button press received - approved"
        else
            echo timeout > /tmp/press.result
            echo "  no button press - not approved"
        fi
        $BB rm -f /tmp/press.request
    fi
    $BB sleep 1
done
