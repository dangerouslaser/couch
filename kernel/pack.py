#!/usr/bin/env python3
"""Replace an HA100 image's kernel, preserving its DTB and boot parameters."""
import argparse
import hashlib
import json
import struct
from pathlib import Path

MAGIC = b'ANDROID!'
FDT_MAGIC = b'\xd0\x0d\xfe\xed'
LIMIT = 16 * 1024 * 1024


def split_dtb(kernel):
    start = 0
    while True:
        at = kernel.find(FDT_MAGIC, start)
        if at < 0:
            raise ValueError('no valid appended DTB ending at the kernel boundary')
        start = at + 4
        if len(kernel) - at < 40:
            continue
        magic, size, structure, strings, reserve, version, compat, cpu, string_size, struct_size = struct.unpack_from('>10I', kernel, at)
        if (size == len(kernel) - at and version >= 17 and compat <= 17
                and 40 <= structure <= size and structure + struct_size <= size
                and 40 <= strings <= size and strings + string_size <= size
                and 40 <= reserve < size and struct_size >= 4
                and kernel[at + structure + struct_size - 4:at + structure + struct_size] == b'\0\0\0\x09'):
            return kernel[:at], kernel[at:]


def repack(image, zimage, ramdisk=None):
    if len(image) < 2048 or image[:8] != MAGIC:
        raise ValueError('not an Android boot image')
    ks, ka, rs, ra, ss, sa, tags, page, version, osver = struct.unpack_from('<10I', image, 8)
    if page not in (2048, 4096, 8192, 16384) or ss or version:
        raise ValueError('only header-v0 images without a second stage are supported')
    align = lambda n: (n + page - 1) // page * page
    rd_at = page + align(ks)
    if not ks or rd_at + rs > len(image):
        raise ValueError('truncated boot image')
    old_kernel = image[page:page + ks]
    _, dtb = split_dtb(old_kernel)
    if len(zimage) < 48 or zimage[36:40] != b'\x18\x28\x6f\x01':
        raise ValueError('not an ARM zImage')
    kernel = zimage + dtb
    rd = image[rd_at:rd_at + rs] if ramdisk is None else ramdisk
    if not rd:
        raise ValueError('empty ramdisk')
    header = bytearray(image[:page])
    struct.pack_into('<I', header, 8, len(kernel))
    struct.pack_into('<I', header, 16, len(rd))
    digest = hashlib.sha1()
    for part in (kernel, rd, b''):
        digest.update(part)
        digest.update(struct.pack('<I', len(part)))
    header[576:608] = digest.digest() + b'\0' * 12
    pad = lambda part: part + b'\0' * (align(len(part)) - len(part))
    output = bytes(header) + pad(kernel) + pad(rd)
    if len(output) > LIMIT:
        raise ValueError('image exceeds the 16 MiB boot partition')
    hashes = {name: hashlib.sha256(data).hexdigest() for name, data in
              [('source_image', image), ('zImage', zimage), ('dtb', dtb), ('ramdisk', rd), ('image', output)]}
    return output, hashes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source')
    parser.add_argument('zimage')
    parser.add_argument('output')
    parser.add_argument('--ramdisk', help='explicit replacement cpio.gz; default preserves source ramdisk')
    args = parser.parse_args()
    out, hashes = repack(Path(args.source).read_bytes(), Path(args.zimage).read_bytes(),
                         Path(args.ramdisk).read_bytes() if args.ramdisk else None)
    Path(args.output).write_bytes(out)
    Path(args.output + '.json').write_text(json.dumps({'bytes': len(out), 'sha256': hashes}, indent=2) + '\n')
    print(f'{args.output}: {len(out)} bytes; sha256 {hashes["image"]}')


if __name__ == '__main__':
    main()
