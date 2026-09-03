#!/usr/bin/env python3
"""Hand-built newc cpio: exact root ownership, real device nodes, correct trailer."""
import os, struct, sys

S_IFDIR, S_IFREG, S_IFCHR, S_IFBLK = 0o040000, 0o100000, 0o020000, 0o060000

class Cpio:
    def __init__(self):
        self.out = bytearray()
        self.ino = 1

    def _entry(self, name, mode, data=b"", rdevmaj=0, rdevmin=0, nlink=1):
        name_b = name.encode() + b"\0"
        hdr = "070701"
        for v in (self.ino, mode, 0, 0, nlink, 0, len(data),
                  0, 0, rdevmaj, rdevmin, len(name_b), 0):
            hdr += "%08X" % v
        self.ino += 1
        self.out += hdr.encode() + name_b
        self.out += b"\0" * ((-len(self.out)) % 4)
        if data:
            self.out += data
            self.out += b"\0" * ((-len(self.out)) % 4)

    def dir(self, name, mode=0o755):   self._entry(name, S_IFDIR | mode, nlink=2)
    def file(self, name, data, mode=0o644): self._entry(name, S_IFREG | mode, data)
    def chardev(self, name, maj, mi, mode=0o600): self._entry(name, S_IFCHR | mode, rdevmaj=maj, rdevmin=mi)
    def blockdev(self, name, maj, mi, mode=0o600): self._entry(name, S_IFBLK | mode, rdevmaj=maj, rdevmin=mi)

    def finish(self):
        self._entry("TRAILER!!!", 0, nlink=1)
        self.out += b"\0" * ((-len(self.out)) % 512)
        return bytes(self.out)

TREE = sys.argv[1] if len(sys.argv) > 1 else "initramfs"
OUT  = sys.argv[2] if len(sys.argv) > 2 else "initramfs.cpio"

c = Cpio()
for d in (".", "bin", "dev", "proc", "sys", "tmp", "etc", "newroot", "mnt"):
    c.dir(d)
c.file("init", open(os.path.join(TREE, "init"), "rb").read(), 0o755)
c.file("bin/busybox", open(os.path.join(TREE, "busybox"), "rb").read(), 0o755)
# Anything else dropped in the tree ships as-is (firmware, modules, tarballs).
c.dir("extra")
for root, _, files in os.walk(os.path.join(TREE, "extra")):
    for f in sorted(files):
        full = os.path.join(root, f)
        rel  = os.path.relpath(full, TREE)
        c.file(rel, open(full, "rb").read(), 0o755)
# Real device nodes, majors read off the running Android. No mknod, no devtmpfs needed.
c.chardev("dev/null",    1,   3, 0o666)
c.chardev("dev/console", 5,   1)
c.chardev("dev/tty",     5,   0, 0o666)
c.chardev("dev/ttyGS0",  233, 0)
c.chardev("dev/ttyMT0",  204, 209)
c.chardev("dev/ttyMT1",  204, 210)
c.chardev("dev/ttyMT2",  204, 211)  # WMT STP transport
c.chardev("dev/fb0",     29,  0)
c.chardev("dev/irtx",    243, 0)
c.blockdev("dev/mmcblk0p8",  179, 8)    # boot: the slot Couch runs from, so a
                                        # running Couch can reflash itself
c.blockdev("dev/mmcblk0p9",  179, 9)    # recovery: Android, the fallback
c.blockdev("dev/mmcblk0p10", 179, 10)   # para: the BCB we must clear
c.blockdev("dev/mmcblk0p11", 179, 11)   # logo: the bootloader splash
c.blockdev("dev/mmcblk0p13", 179, 13)   # expdb: marker and log scratch
c.blockdev("dev/mmcblk0p14", 179, 14)   # vendor: MTK modules, firmware, wpa_supplicant
c.blockdev("dev/mmcblk0p21", 179, 21)   # system: bionic + the dynamic linker
c.blockdev("dev/mmcblk0p22", 179, 22)   # cache: future Alpine rootfs
c.blockdev("dev/mmcblk0p23", 179, 23)   # userdata: stored wifi credentials
c.chardev("dev/urandom", 1, 9, 0o666)
c.chardev("dev/random",  1, 8, 0o666)
c.chardev("dev/zero",    1, 5, 0o666)
open(OUT, "wb").write(c.finish())
print("cpio:", OUT, os.path.getsize(OUT), "bytes")
