#!/usr/bin/env python3
"""Patch a property in the device tree appended to the kernel image.

The keypad's behaviour is fixed in the DTB, not in any driver tunable:
debounce-delay-ms = 50 makes the driver swallow rapid taps (a double-tap cycle
falls inside the window), and linux,no-autorepeat disables held-key repeat.

The DTB is appended to the zImage, so it travels with the boot image we already
build. This walks the flattened tree properly rather than blind-searching for
bytes, so it patches the right property in the right node.

  dtbpatch.py <kernel.bin> <out.bin> <node> <prop> <u32 value>
"""
import struct, sys

FDT_BEGIN_NODE, FDT_END_NODE, FDT_PROP, FDT_NOP, FDT_END = 1, 2, 3, 4, 9


def find_dtb(blob):
    off = blob.find(b"\xd0\x0d\xfe\xed")
    if off < 0:
        sys.exit("no DTB magic in image")
    size = struct.unpack(">I", blob[off + 4:off + 8])[0]
    return off, size


def patch(dtb, want_node, want_prop, value):
    magic, totalsize, off_struct, off_strings = struct.unpack(">4I", dtb[:16])
    assert magic == 0xd00dfeed
    out = bytearray(dtb)
    p = off_struct
    path = []
    hits = []
    while p < off_struct + totalsize:
        (tag,) = struct.unpack(">I", dtb[p:p + 4]); p += 4
        if tag == FDT_BEGIN_NODE:
            end = dtb.index(b"\0", p)
            path.append(dtb[p:end].decode("ascii", "replace"))
            p = (end + 4) & ~3
        elif tag == FDT_END_NODE:
            if path: path.pop()
            p = (p + 3) & ~3
        elif tag == FDT_PROP:
            length, nameoff = struct.unpack(">II", dtb[p:p + 8]); p += 8
            nend = dtb.index(b"\0", off_strings + nameoff)
            name = dtb[off_strings + nameoff:nend].decode("ascii", "replace")
            if path and path[-1] == want_node and name == want_prop:
                old = struct.unpack(">I", dtb[p:p + 4])[0] if length == 4 else None
                hits.append((p, length, old))
            p = (p + length + 3) & ~3
        elif tag in (FDT_NOP,):
            continue
        elif tag == FDT_END:
            break
        else:
            sys.exit(f"bad tag {tag} at {p}")
    if not hits:
        sys.exit(f"{want_node}/{want_prop} not found")
    for off, length, old in hits:
        if length != 4:
            sys.exit(f"{want_prop} is {length} bytes, expected 4")
        out[off:off + 4] = struct.pack(">I", value)
        print(f"  {want_node}/{want_prop}: {old} -> {value}")
    return bytes(out)


if __name__ == "__main__":
    if len(sys.argv) != 6:
        sys.exit(__doc__)
    src, dst, node, prop, val = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5])
    blob = open(src, "rb").read()
    off, size = find_dtb(blob)
    print(f"DTB at 0x{off:x}, {size} bytes")
    new_dtb = patch(blob[off:off + size], node, prop, val)
    assert len(new_dtb) == size, "patched DTB changed size"
    open(dst, "wb").write(blob[:off] + new_dtb + blob[off + size:])
    print(f"wrote {dst}")
