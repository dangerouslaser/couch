#!/usr/bin/env python3
"""Unpack and repack a MediaTek logo partition.

Layout: a 512-byte MTK image header (magic 0x58881688, then the payload size
and the partition name), followed by a table of [count, block_size, offsets...]
and then that many zlib streams, each one a raw RGB565 frame. Offsets are
relative to the start of the table, and the table's own length is exactly
offsets[0], which is how the first image is found.
"""
import struct, sys, zlib

HDR = 512
MAGIC = 0x58881688


def unpack(path):
    data = open(path, "rb").read()
    magic, size, name = struct.unpack_from("<II32s", data, 0)
    if magic != MAGIC:
        sys.exit("not a MTK image: magic %#x" % magic)
    count, block = struct.unpack_from("<II", data, HDR)
    offs = list(struct.unpack_from("<%dI" % count, data, HDR + 8))
    ends = offs[1:] + [block]
    blobs = [data[HDR + a:HDR + b] for a, b in zip(offs, ends)]
    return name.split(b"\0")[0].decode(), blobs


def pack(name, blobs):
    count = len(blobs)
    table = 8 + 4 * count
    offs, cur = [], table
    for b in blobs:
        offs.append(cur)
        cur += len(b)
    block = cur
    out = bytearray(struct.pack("<II32s", MAGIC, block, name.encode()))
    out += b"\0" * (HDR - len(out))
    out += struct.pack("<II", count, block)
    out += b"".join(struct.pack("<I", o) for o in offs)
    out += b"".join(blobs)
    return bytes(out)


if __name__ == "__main__":
    name, blobs = unpack(sys.argv[1])
    print("partition %r, %d images" % (name, len(blobs)))
    for i, b in enumerate(blobs):
        try:
            raw = zlib.decompress(b)
            px = len(raw) // 2
            dims = "%dx%d" % (480, px // 480) if px % 480 == 0 else "?"
            print("  [%2d] %7d -> %8d bytes  %s" % (i, len(b), len(raw), dims))
        except zlib.error as e:
            print("  [%2d] %7d  not zlib (%s)" % (i, len(b), e))
