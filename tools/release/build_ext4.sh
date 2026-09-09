#!/bin/sh
# Create only /out/userdata.ext4, a regular file in a fresh output directory.
set -eu
export SOURCE_DATE_EPOCH="$1" E2FSPROGS_FAKE_TIME="$1"
BLOCKS="$2"
UUID="$3"
cd /tools
sha256sum -c /input/tool-hashes.txt > /out/tool-checks.txt
apk --repositories-file /dev/null --keys-dir /usr/share/apk/keys/x86_64 verify packages/*.apk >> /out/tool-checks.txt
mkdir -p /tmp/tools/etc/apk /tmp/tools/lib/apk/db /tmp/tools/var/cache/apk /tmp/tools/tmp /tmp/root
touch /tmp/tools/etc/apk/world /tmp/tools/lib/apk/db/installed /tmp/tools/lib/apk/db/triggers
dd if=/dev/zero of=/tmp/tools/lib/apk/db/scripts.tar bs=1024 count=1 2>/dev/null
apk --root /tmp/tools --arch x86_64 --keys-dir /usr/share/apk/keys/x86_64 \
    --repositories-file /dev/null --no-network --no-scripts --initdb --force-non-repository \
    add /tools/packages/*.apk > /out/tool-install.txt 2>&1
cd /tmp/root
tar -xzf /input/rootfs-staging.tar.gz
export LD_LIBRARY_PATH=/tmp/tools/lib:/tmp/tools/usr/lib
export MKE2FS_CONFIG=/input/mke2fs.conf
MK=/tmp/tools/sbin
[ ! -e /out/userdata.ext4 ] && [ ! -L /out/userdata.ext4 ]
"$MK/mke2fs" -V > /out/tool-version.txt 2>&1
# Explicit legacy-compatible feature set: no metadata_csum, 64bit or orphan_file.
# The new regular file contains raw ext4 bytes, never Android sparse encoding.
"$MK/mke2fs" -F -t ext4 -T default -b 4096 -I 256 -i 16384 -m 0 \
    -U "$UUID" -L couch-data \
    -O none,has_journal,ext_attr,dir_index,filetype,extent,sparse_super,large_file,uninit_bg,dir_nlink,extra_isize \
    -E nodiscard,lazy_itable_init=0,lazy_journal_init=0,hash_seed="$UUID",root_owner=0:0 \
    -d /tmp/root /out/userdata.ext4 "$BLOCKS" > /out/mke2fs.txt 2>&1
"$MK/e2fsck" -fn /out/userdata.ext4 > /out/e2fsck.txt 2>&1
/tmp/tools/usr/sbin/dumpe2fs -h /out/userdata.ext4 > /out/superblock.txt 2>&1
