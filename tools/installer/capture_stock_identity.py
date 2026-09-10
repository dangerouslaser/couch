#!/usr/bin/env python3
"""Private known-device identity recapture after stock Android startup; no flash writes."""
import argparse
import hashlib
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace

from capture_readonly import baseline_record, capture
from couch_install import (IDENTITY_PARTITIONS, MODEL, REPO, InstallError, digest,
                           layout, read_json, regular, require, save_json)


def recapture(args, *, capture_fn=capture):
    baseline_path = regular(args.baseline)
    baseline = baseline_record(baseline_path)
    require(baseline is not None and 'cid' in baseline and baseline.get('identity_sha256'),
            'A retained complete trusted device baseline is required')
    cid_hash = hashlib.sha256(bytes.fromhex(baseline['cid'])).hexdigest()
    require(args.confirm_cid_sha256 == cid_hash, 'Explicit trusted target CID confirmation mismatch')
    profile_path = regular(args.stock_manifest)
    require(digest(profile_path) == args.stock_manifest_sha256, 'Reviewed stock manifest pin mismatch')
    profile = read_json(profile_path)
    require(profile.get('schema') == 1 and profile.get('model') == MODEL
            and profile.get('private_candidate') is True and profile.get('installable') is False
            and profile.get('purpose') == 'stock-baseline', 'Expected reviewed private stock-baseline profile')
    require(layout(profile['partitions']) == baseline['partitions'], 'Stock profile differs from trusted device layout')
    destination = args.backup_dir.absolute()
    require(not destination.is_symlink() and not destination.exists(), 'Use a new stock identity destination')
    require(not destination.resolve().is_relative_to(REPO), 'Keep identity backups outside the repository')
    predecessor = digest(baseline_path)
    # The retained canonical CID is accepted only after the DA observes an exact
    # fixed-encoding hash match. Mutable partition bytes deliberately get a new
    # snapshot; no claim is made that Android preserved their previous hashes.
    binding = {key: baseline[key] for key in ('schema', 'capacity', 'partitions', 'cid')}
    with tempfile.TemporaryDirectory(prefix='couch-stock-binding-') as temporary:
        binding_path = Path(temporary) / 'binding.json'
        save_json(binding_path, binding)
        values = vars(args).copy()
        values.update(baseline=binding_path, boot_after_capture=False)
        capture_fn(SimpleNamespace(**values))
    if args.check_only:
        return
    report = read_json(destination / 'readback.json')
    require(report.get('complete') is True and report.get('usb_cleanup_verified') is True
            and report.get('runtime_cid_verified') is True, 'Capture and USB cleanup must complete before creating a new baseline')
    observed = report['device']
    require(observed.get('runtime_cid_sha256') == cid_hash
            and observed.get('cid_encoding') == 'mt6580-legacy-le32-registers'
            and observed.get('partitions') == baseline['partitions']
            and observed.get('capacity') == baseline['capacity'], 'Captured device differs from trusted binding')
    hashes = report['backups']
    require(set(hashes) == IDENTITY_PARTITIONS, 'Incomplete fresh identity capture')
    for name, checksum in hashes.items():
        image = regular(destination / f'{name}.img')
        require(image.stat().st_size == baseline['partitions'][name]['size']
                and digest(image) == checksum, f'Captured identity file differs: {name}')
    result = {**binding, 'identity_sha256': hashes, 'source': 'known-device-stock-usb-recapture',
              'predecessor_sha256': predecessor, 'stock_manifest_sha256': args.stock_manifest_sha256,
              'loader_sha256': args.loader_sha256, 'board_data_sha256': args.preloader_sha256,
              'identity_decoded': False, 'normal_os_verified': False}
    output = destination / 'baseline.json'
    require(not output.exists() and not output.is_symlink(), 'Never replace an existing baseline')
    save_json(output, result)
    print('New USB identity baseline saved separately; original baseline retained. Vendor identity decoding and OS startup are not verified.')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('baseline', 'stock-manifest', 'checkout', 'loader', 'preloader', 'backup-dir'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('stock-manifest-sha256', 'confirm-cid-sha256', 'loader-sha256', 'preloader-sha256', 'ports'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--bus', type=int, required=True)
    parser.add_argument('--timeout', type=float, default=60)
    parser.add_argument('--lock-dir', type=Path, default=Path.home() / '.local/state/couch-installer/locks')
    parser.add_argument('--check-only', action='store_true')
    try:
        recapture(parser.parse_args(argv))
        return 0
    except (InstallError, OSError, ValueError, KeyError, TypeError) as error:
        print(f'Stock identity recapture stopped: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
