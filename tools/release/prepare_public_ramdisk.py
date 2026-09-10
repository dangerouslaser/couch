#!/usr/bin/env python3
"""Prepare a vendor-free RAM payload, never an Android boot image or device write."""
import argparse
import gzip
import json
from pathlib import Path

from clean_stage import require
from prepare_wifi_ramdisk import neutral_files, ramdisk, sha
from runtime_inventory import cpio_files


def prepare(busybox, service, apk_cache, filesystem_cache, display, wmt_properties, output):
    require(not output.exists(), 'Public RAM output must be new')
    files, apk_hash, fs_hash = neutral_files(
        busybox, service, apk_cache, installer=True, display=display,
        wmt_properties=wmt_properties, filesystem_cache=filesystem_cache)
    # Android owner paths may appear as directory/symlink scaffolding in newc,
    # but this payload must contain none of the compiled proprietary file bytes.
    pin = json.loads((Path(__file__).parent / 'ha100_official_runtime.json').read_text())
    owner_names = {record['path'] for record in pin['files']}
    owner_hashes = {record['sha256'] for record in pin['files']}
    require(not owner_names.intersection(files), 'Owner path in neutral RAM')
    require(not owner_hashes.intersection(map(sha, files.values())), 'Owner content in neutral RAM')
    raw = ramdisk(files)
    cpio_files(raw)
    data = gzip.compress(raw, mtime=0)
    require(len(raw) <= 64 * 1024**2 and len(data) <= 16 * 1024**2, 'RAM payload exceeds bounds')
    result = {'schema': 1, 'kind': 'couch-owner-neutral-ramdisk',
              'file': 'installer.cpio.gz', 'size': len(data), 'sha256': sha(data),
              'owner_vendor_source_sha256': pin['sha256'],
              'apk_inventory_sha256': apk_hash, 'filesystem_inventory_sha256': fs_hash,
              'files': {name: {'size': len(value), 'sha256': sha(value)} for name, value in sorted(files.items())},
              'bootable': False, 'physical_boot_verified': False,
              'assembly': 'Native host inserts pinned owner files and assembles an owner-local Android header/kernel/DTB image.'}
    output.mkdir(parents=True, mode=0o700)
    (output / 'installer.cpio.gz').write_bytes(data)
    (output / 'ramdisk.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('busybox', 'service', 'apk-cache', 'filesystem-cache', 'display', 'wmt-properties', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.busybox, args.service, args.apk_cache, args.filesystem_cache,
                     args.display, args.wmt_properties, args.output)
    print(f"Prepared vendor-free RAM payload: {result['size']} bytes")
