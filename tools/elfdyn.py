#!/usr/bin/env python3
"""readelf -d for 32-bit ARM ELF.

The device has busybox and the host has no binutils, but the dynamic section is
what decides whether a libhybris port is realistic - so parse it directly.

.dynstr is found through the section headers where they exist, and through the
PT_LOAD mapping where they do not: vendor blobs are frequently stripped of
section headers, libGLES_mali.so among them.
"""
import struct
import sys

DT = {1: "NEEDED", 12: "INIT", 13: "FINI", 14: "SONAME", 15: "RPATH",
      25: "INIT_ARRAY", 26: "FINI_ARRAY", 29: "RUNPATH", 30: "FLAGS",
      0x6ffffffb: "FLAGS_1", 0x6ffffffe: "VERNEED"}


def dynstr_offset(d, e_phoff, e_phentsize, e_phnum, dt_strtab):
    """File offset of .dynstr: section headers first, then PT_LOAD."""
    e_shoff, = struct.unpack_from("<I", d, 0x20)
    e_shentsize, e_shnum, e_shstrndx = struct.unpack_from("<HHH", d, 0x2e)
    if e_shoff and e_shnum and e_shstrndx < e_shnum:
        sh = e_shoff + e_shstrndx * e_shentsize
        shstr, = struct.unpack_from("<I", d, sh + 0x10)
        for i in range(e_shnum):
            o = e_shoff + i * e_shentsize
            name, = struct.unpack_from("<I", d, o)
            end = d.index(b"\0", shstr + name)
            if d[shstr + name:end] == b".dynstr":
                off, = struct.unpack_from("<I", d, o + 0x10)
                return off
    if dt_strtab is None:
        return None
    for i in range(e_phnum):
        q = e_phoff + i * e_phentsize
        p_type, p_offset, p_vaddr, p_filesz = struct.unpack_from("<IIII", d, q)
        if p_type == 1 and p_vaddr <= dt_strtab < p_vaddr + p_filesz:
            return p_offset + (dt_strtab - p_vaddr)
    return dt_strtab


def dump(path):
    d = open(path, "rb").read()
    if d[:4] != b"\x7fELF" or d[4] != 1:
        print(f"{path}: not a 32-bit ELF")
        return
    e_phoff, = struct.unpack_from("<I", d, 0x1c)
    e_phentsize, e_phnum = struct.unpack_from("<HH", d, 0x2a)

    dyn = None
    for i in range(e_phnum):
        q = e_phoff + i * e_phentsize
        p_type, p_offset, _, p_filesz = struct.unpack_from("<IIII", d, q)
        if p_type == 2:
            dyn = (p_offset, p_filesz)
    if dyn is None:
        print(f"{path}: no PT_DYNAMIC")
        return

    entries, dt_strtab = [], None
    for off in range(dyn[0], dyn[0] + dyn[1], 8):
        tag, val = struct.unpack_from("<II", d, off)
        if tag == 0:
            break
        entries.append((tag, val))
        if tag == 5:
            dt_strtab = val

    strtab = dynstr_offset(d, e_phoff, e_phentsize, e_phnum, dt_strtab)
    if strtab is None or strtab >= len(d):
        print(f"{path}: could not locate .dynstr")
        return

    def s(idx):
        end = d.index(b"\0", strtab + idx)
        return d[strtab + idx:end].decode("utf-8", "replace")

    print(f"\n=== {path} ===")
    for tag, val in entries:
        if tag in (14, 15, 29):
            print(f"  {DT[tag]:<10} {s(val)}")
    needed = [s(v) for t, v in entries if t == 1]
    print(f"  NEEDED     ({len(needed)})")
    for n in needed:
        print(f"      {n}")


for path in sys.argv[1:]:
    dump(path)
