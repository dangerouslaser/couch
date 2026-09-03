#!/bin/busybox sh
# Start sshd, but only if the user has deliberately set up a way in.
#
# A shipped image trusts nobody: no enrolled key and no root password means no
# sshd at all, so a remote straight off the flasher exposes no remote entry
# point. Both credentials are established through the setup portal, and both are
# gated on a physical button press (see confirm.sh).
#
# Runs inside the Alpine chroot - stage2 calls it via chroot, join.sh directly.
BB=/bin/busybox
KEY=/root/.ssh/authorized_keys

# A locked account carries "!" or "*" in the password field of /etc/shadow; a
# real hash starts with "$".
HASH=$($BB grep "^root:" /etc/shadow 2>/dev/null | $BB cut -d: -f2)
case "$HASH" in
    \$*) PW=yes ;;
    *)   PW=no  ;;
esac
[ -s "$KEY" ] && KEYS=yes || KEYS=no

if [ "$PW" = no ] && [ "$KEYS" = no ]; then
    echo "sshd not started: no key and no password (use the setup portal)"
    exit 0
fi

# Host keys are generated on first use rather than baked into the image: baked
# keys would be identical on every device that ever flashed it.
[ -f /etc/ssh/ssh_host_ed25519_key ] || /usr/bin/ssh-keygen -A >/dev/null 2>&1

$BB pidof sshd >/dev/null && exit 0
/usr/sbin/sshd 2>/dev/null && echo "sshd listening (key=$KEYS password=$PW)"
