#!/bin/sh
# Shared paths. The partition backup is deliberately outside the repo: it is 1.7GB
# and contains per-device calibration (nvram/nvdata/protect1/protect2) that must
# never be committed or shared.
BACKUP_DIR="${BACKUP_DIR:-$HOME/Projects/sanytron-device-backup}"
BUILD_DIR="${BUILD_DIR:-$(dirname "$0")/../build}"
ADB="${ADB:-adb}"

# Partition map, read off the device (by-name symlinks under
# /dev/block/platform/mtk-msdc.0/11120000.msdc0/by-name/).
# The slots were swapped at install time: Couch boots from "boot". "recovery"
# held Android's boot image for a while; since the kernel work it holds the
# Couch recovery image (tools/build-recovery.sh): stock kernel, USB shell,
# WiFi and sshd, no UI - the thing a bad kernel resets into. Android's boot
# image exists only in the backup now (android-p9-BACKUP.img).
# Note these are Android's /dev/block paths, for the adb-era tools. Under
# Couch the same partitions are /dev/mmcblk0pN.
PART_BOOT=/dev/block/mmcblk0p8         # Couch
PART_RECOVERY=/dev/block/mmcblk0p9     # Couch recovery (was Android)
PART_PARA=/dev/block/mmcblk0p10        # BCB / misc
PART_CACHE=/dev/block/mmcblk0p22       # 112MB, expendable
PART_ROOTFS=/dev/block/mmcblk0p23      # 5.5GB userdata: where Couch lives
PART_EXPDB=/dev/block/mmcblk0p13       # 10MB MTK crash dumps: our marker/log scratch

# Markers and kernel logs live in expdb, not the recovery slot: after the slot
# swap, recovery holds Android's boot image and must not be scribbled on.
# Boot markers and the kernel-log snapshot. These sit above MTK's crash dump
# (its sections end around 6.3MB of expdb) so that a watchdog reset cannot
# overwrite the record of the boot that caused it. Keep in step with BASE and
# DMESG_SECTOR in initramfs/init.
MARKER_SECTOR=13000
DMESG_SECTOR=13064
DMESG2_SECTOR=15200                    # second snapshot, also above the dump
