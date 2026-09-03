#!/bin/sh
# Shared paths. The partition backup is deliberately outside the repo: it is 1.7GB
# and contains per-device calibration (nvram/nvdata/protect1/protect2) that must
# never be committed or shared.
BACKUP_DIR="${BACKUP_DIR:-$HOME/Projects/sanytron-device-backup}"
BUILD_DIR="${BUILD_DIR:-$(dirname "$0")/../build}"
ADB="${ADB:-adb}"

# Partition map, read off the device (by-name symlinks under
# /dev/block/platform/mtk-msdc.0/11120000.msdc0/by-name/).
PART_BOOT=/dev/block/mmcblk0p8
PART_RECOVERY=/dev/block/mmcblk0p9     # our Linux slot; Android's boot is untouched
PART_PARA=/dev/block/mmcblk0p10        # BCB / misc
PART_CACHE=/dev/block/mmcblk0p22       # 112MB, expendable: the Alpine rootfs target
PART_EXPDB=/dev/block/mmcblk0p13       # 10MB MTK crash dumps: our marker/log scratch

# Markers and kernel logs live in expdb, not the recovery slot: after the slot
# swap, recovery holds Android's boot image and must not be scribbled on.
MARKER_SECTOR=0
DMESG_SECTOR=2048                      # 1MB in
DMESG2_SECTOR=6144                     # 3MB in
