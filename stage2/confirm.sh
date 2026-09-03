#!/bin/busybox sh
# Watch for enrolment requests from the portal and require a physical button
# press to approve them.
#
# A request alone must never be enough to add an SSH key: this proves someone is
# holding the remote. Any key on the keypad counts - reading 16 bytes from the
# evdev node blocks until an input event arrives (struct input_event is 16 bytes
# on 32-bit ARM).
BB=/bin/busybox
while true; do
    if [ -f /tmp/press.request ]; then
        echo ""
        echo "  >>> PRESS ANY BUTTON ON THE REMOTE TO AUTHORISE SSH KEY <<<"
        for ev in /dev/input/event1 /dev/input/event2 /dev/input/event0; do
            [ -e "$ev" ] || continue
            if $BB timeout 25 $BB dd if="$ev" bs=16 count=1 >/dev/null 2>&1; then
                : > /tmp/press.ok
                echo "  button press received - key authorised"
                break
            fi
        done
        $BB rm -f /tmp/press.request
    fi
    $BB sleep 1
done
