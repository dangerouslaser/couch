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

# Marker area in the tail of the recovery partition, past our ~7.7MB image.
MARKER_SECTOR=24576                    # 12MB
DMESG_SECTOR=26624                     # 13MB
DMESG2_SECTOR=28672                    # 14MB
