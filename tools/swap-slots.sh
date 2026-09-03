#!/bin/sh
# Make Linux the default OS.
#
#   boot     <- our Linux image   (Normal Boot in lk's menu)
#   recovery <- Android's boot.img (Recovery Mode in lk's menu)
#
# lk's menu becomes an OS picker, so Android is always reachable from the device
# itself with no cable: hold Volume Up at power-on, Volume Up moves the selector,
# Volume Down selects, choose [Recovery Mode].
#
# Run from Android (adb). Refuses unless the backup of Android's boot image is
# present and valid, because that backup is the only way back.
set -e
cd "$(dirname "$0")/.."
. tools/env.sh
BB=/data/local/tmp/busybox

ANDROID_BOOT="$BACKUP_DIR/boot.img"
LINUX_IMG=build/linux-recovery.img

[ -f "$ANDROID_BOOT" ] || { echo "missing $ANDROID_BOOT - run tools/backup.sh first"; exit 1; }
[ -f "$LINUX_IMG" ]    || { echo "missing $LINUX_IMG - run tools/build.sh first"; exit 1; }

# Both must be real Android boot images or we are about to brick the boot path.
for f in "$ANDROID_BOOT" "$LINUX_IMG"; do
    magic=$(head -c 8 "$f")
    [ "$magic" = "ANDROID!" ] || { echo "$f is not an Android boot image (magic '$magic')"; exit 1; }
done
echo "verified: both images carry the ANDROID! magic"

sh "$(dirname "$0")/wait-adb.sh" 60 || exit 1
$ADB shell "[ -x $BB ]" 2>/dev/null || { $ADB push build/busybox-armv7l $BB >/dev/null; $ADB shell "chmod 755 $BB"; }

echo "writing Android's boot.img -> recovery ..."
$ADB push "$ANDROID_BOOT" /data/local/tmp/android-boot.img >/dev/null
$ADB shell "$BB dd if=/data/local/tmp/android-boot.img of=$PART_RECOVERY bs=1048576 conv=notrunc 2>&1 | tail -1; sync"

echo "writing Linux -> boot ..."
$ADB push "$LINUX_IMG" /data/local/tmp/linux-boot.img >/dev/null
$ADB shell "$BB dd if=/data/local/tmp/linux-boot.img of=$PART_BOOT bs=1048576 conv=notrunc 2>&1 | tail -1; sync"

echo "verifying ..."
$ADB exec-out "dd if=$PART_BOOT bs=1048576 count=8" 2>/dev/null > build/verify-boot.bin
$ADB exec-out "dd if=$PART_RECOVERY bs=1048576 count=16" 2>/dev/null > build/verify-recovery.bin
python3 - "$LINUX_IMG" "$ANDROID_BOOT" <<'PY'
import sys, hashlib
def h(p, n): return hashlib.sha1(open(p,"rb").read()[:n]).hexdigest()
lin, andr = sys.argv[1], sys.argv[2]
nl = len(open(lin,"rb").read()); na = len(open(andr,"rb").read())
ok_l = h("build/verify-boot.bin", nl) == h(lin, nl)
ok_a = h("build/verify-recovery.bin", na) == h(andr, na)
print(f"  boot     holds Linux   : {ok_l}")
print(f"  recovery holds Android : {ok_a}")
sys.exit(0 if (ok_l and ok_a) else 1)
PY

# Clear the BCB so the very next boot takes the boot slot, i.e. Linux.
$ADB shell "$BB dd if=/dev/zero of=$PART_PARA bs=512 count=1 conv=notrunc 2>/dev/null; sync"
echo "linux-in-boot" > build/layout
echo
echo "done. Normal Boot = Linux, Recovery Mode = Android."
echo "back to Android:  tools/boot-android.sh   (or lk menu: hold Vol Up at power-on)"
