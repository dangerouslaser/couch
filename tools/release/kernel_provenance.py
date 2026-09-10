#!/usr/bin/env python3
"""Verify a staged boot kernel against the tested candidate; never enable installs."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'kernel'))
from pack import split_dtb

PIN = Path(__file__).resolve().parents[2] / 'kernel/release-pin.json'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def boot_kernel(data, pin):
    if len(data) < 2048 or data[:8] != b'ANDROID!':
        raise ValueError('Invalid Android boot image')
    size, _, rd_size, _, second, _, _, page, version = struct.unpack_from('<9I', data, 8)
    if page not in (2048, 4096, 8192, 16384) or version or second or not rd_size:
        raise ValueError('Unsupported boot image layout')
    end = page + ((size + page - 1) // page) * page + rd_size
    if not size or end > len(data):
        raise ValueError('Truncated boot image')
    zimage, dtb = split_dtb(data[page:page+size])
    if len(zimage) < 48 or zimage[36:40] != b'\x18\x28\x6f\x01':
        raise ValueError('Missing ARM zImage magic')
    if sha(zimage) != pin['zimage_sha256']:
        raise ValueError('Boot kernel differs from the tested release candidate')
    return {'source_commit': pin['source_commit'], 'zimage_sha256': sha(zimage),
            'dtb_sha256': sha(dtb), 'installable': False,
            'status': 'kernel matches tested candidate; remaining installer gates still apply'}


def verify_manifest(manifest, pin):
    expected = {'profile': pin['profile'], 'source_commit': pin['source_commit'],
                'source_status': '', 'source_diff_sha256': sha(b''),
                'compiler_sha256': pin['compiler_sha256'], 'container': pin['container']}
    for name, value in expected.items():
        if manifest.get(name) != value:
            raise ValueError(f'Kernel manifest mismatch: {name}')
    hashes = manifest.get('sha256', {})
    if hashes.get('.config') != pin['config_sha256'] or hashes.get('arch/arm/boot/zImage') != pin['zimage_sha256']:
        raise ValueError('Kernel config or artifact manifest hash mismatch')


def verify(data, manifest, pin):
    verify_manifest(manifest, pin)
    return boot_kernel(data, pin)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--boot', required=True, type=Path)
    parser.add_argument('--kernel-manifest', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(args.boot.read_bytes(), json.loads(args.kernel_manifest.read_text()),
                            json.loads(PIN.read_text())), indent=2))


if __name__ == '__main__':
    main()
