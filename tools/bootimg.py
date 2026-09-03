#!/usr/bin/env python3
"""Unpack and pack MediaTek/Android boot images (header v0, 2048-byte pages).

The HA100's stock image is unsigned - 8.7MB of trailing zeros where a signature
block would be - which is why lk accepts anything we build here.
"""
import struct, hashlib, sys, os

PAGE   = 2048
KADDR  = 0x80008000
RADDR  = 0x84000000
SADDR  = 0x80f00000
TAGS   = 0x8e000000
CMDLINE = b"bootopt=64S3,32S1,32S1 buildvariant=userdebug"


def unpack(path, outdir):
    d = open(path, "rb").read()
    (magic, ksz, kaddr, rsz, raddr, ssz, saddr,
     tags, pagesz, hdrv, osver) = struct.unpack("<8sIIIIIIIIII", d[:48])
    assert magic == b"ANDROID!", "not an Android boot image"
    os.makedirs(outdir, exist_ok=True)
    pgs = lambda n: (n + pagesz - 1) // pagesz
    off = pagesz
    open(os.path.join(outdir, "kernel.bin"), "wb").write(d[off:off + ksz])
    off += pgs(ksz) * pagesz
    open(os.path.join(outdir, "ramdisk.cpio.gz"), "wb").write(d[off:off + rsz])
    print("kernel  %d bytes @ 0x%08x" % (ksz, kaddr))
    print("ramdisk %d bytes @ 0x%08x" % (rsz, raddr))
    print("page %d  header v%d  os_version 0x%x" % (pagesz, hdrv, osver))
    print("cmdline %r" % d[64:64 + 512].rstrip(b"\0"))
    return osver


def pack(kernel_path, ramdisk_path, out, osver=0, cmdline=CMDLINE):
    kernel  = open(kernel_path, "rb").read()
    ramdisk = open(ramdisk_path, "rb").read()
    sha = hashlib.sha1()
    for blob in (kernel, ramdisk, b""):
        sha.update(blob)
        sha.update(struct.pack("<I", len(blob)))
    hdr  = b"ANDROID!"
    hdr += struct.pack("<IIIIIIIIII", len(kernel), KADDR, len(ramdisk), RADDR,
                       0, SADDR, TAGS, PAGE, 0, osver)
    hdr += b"\0" * 16
    hdr += cmdline.ljust(512, b"\0")
    hdr += sha.digest() + b"\0" * 12
    hdr += b"\0" * 1024
    hdr  = hdr.ljust(PAGE, b"\0")
    pad  = lambda b: b + b"\0" * ((-len(b)) % PAGE)
    img  = hdr + pad(kernel) + pad(ramdisk)
    open(out, "wb").write(img)
    print("%s: %d bytes (%.1f MB), kernel %d, ramdisk %d"
          % (out, len(img), len(img) / 1048576, len(kernel), len(ramdisk)))
    if len(img) > 16 * 1024 * 1024:
        sys.exit("ERROR: larger than the 16MB recovery partition")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    if sys.argv[1] == "unpack":
        unpack(sys.argv[2], sys.argv[3])
    elif sys.argv[1] == "pack":
        osver = 0
        if os.path.exists(sys.argv[5] if len(sys.argv) > 5 else ""):
            osver = struct.unpack("<I", open(sys.argv[5], "rb").read(48)[44:48])[0]
        pack(sys.argv[2], sys.argv[3], sys.argv[4], osver)
    else:
        sys.exit(__doc__)
