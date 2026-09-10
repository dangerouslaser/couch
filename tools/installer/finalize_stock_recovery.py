#!/usr/bin/env python3
"""Private recovery-only finalization after the operator verifies stock Android boots."""
import argparse
import hashlib
import re
from pathlib import Path
import sys

from capture_readonly import baseline_record, compare_baseline, wait_preloader
from couch_install import (MODEL, REPO, InstallError, digest, fingerprint, layout,
                           read_json, regular, require, save_json, sync_directory)
from mtk_readonly import ConnectedMtkReader
from mtk_session import connected_session, loader_bytes, source_pin
from mtk_usb import ExactUsbBackend
from mtk_writer import ConnectedMtkWriter


def validate(args):
    require(args.allow_private_flash and args.stock_boot_confirmed,
            'Both --allow-private-flash and --stock-boot-confirmed are required')
    baseline = baseline_record(args.baseline)
    require(baseline and 'cid' in baseline and baseline.get('identity_sha256'), 'Retained trusted device baseline required')
    require(args.confirm_cid_sha256 == hashlib.sha256(bytes.fromhex(baseline['cid'])).hexdigest(),
            'Explicit target CID confirmation mismatch')
    profile = regular(args.stock_manifest)
    require(digest(profile) == args.stock_manifest_sha256, 'Reviewed stock manifest pin mismatch')
    value = read_json(profile)
    require(value.get('schema') == 1 and value.get('model') == MODEL
            and value.get('private_candidate') is True and value.get('installable') is False
            and value.get('purpose') == 'stock-baseline', 'Expected private reviewed stock profile')
    require(layout(value['partitions']) == baseline['partitions'], 'Stock profile differs from trusted layout')
    retained = {name: value['images'][name]['sha256'] for name in ('boot', 'odmdtbo')}
    require(all(isinstance(item, str) and re.fullmatch(r'[0-9a-f]{64}', item) for item in retained.values()),
            'Invalid pinned stock boot/odmdtbo hashes')
    image = regular(args.recovery).resolve()
    require(image.stat().st_size == baseline['partitions']['recovery']['size'], 'Recovery must be an exact full-partition raw image')
    with image.open('rb') as stream:
        require(stream.read(8) == b'ANDROID!', 'Original recovery lacks Android boot image magic')
    require(digest(image) == args.recovery_sha256, 'Original recovery hash mismatch')
    destination = args.backup_dir.absolute()
    require(not destination.exists() and not destination.is_symlink(), 'Use a new finalization destination; resume is disabled')
    require(not destination.resolve().is_relative_to(REPO), 'Keep identity and journals outside Git')
    ports = tuple(int(part) for part in args.ports.split('.'))
    require(args.bus > 0 and ports and all(part > 0 for part in ports), 'Invalid USB bus/port')
    require(0 < args.timeout <= 600, 'Timeout must be between 1 and 600 seconds')
    source_pin(args.checkout)
    loader_bytes(args.loader, args.loader_sha256)
    loader_bytes(args.preloader, args.preloader_sha256)
    release = {'schema': 1, 'model': MODEL, 'partitions': baseline['partitions'],
               'retained_stock': retained, 'images': {'recovery': {'file': image.name, 'sha256': args.recovery_sha256}}}
    return baseline, release, image.parent, destination, ports


def run(args, *, session=connected_session, reader_factory=ConnectedMtkReader, writer_factory=ConnectedMtkWriter):
    baseline, release, bundle, destination, ports = validate(args)
    if args.check_only:
        print('Trusted CID/layout, stock profile, recovery bytes and source pins verified. No USB operation performed.')
        return
    binding_baseline = {key: baseline[key] for key in ('schema', 'cid', 'capacity', 'partitions')}
    record = None
    fresh = None

    def choose(enumerate_devices):
        print('Waiting for the explicitly selected preloader; no reset will be sent.', flush=True)
        return wait_preloader(enumerate_devices, args.bus, ports, args.timeout)

    def device_factory(mtk, revision):
        nonlocal record, fresh
        reader = reader_factory(mtk, revision)
        compare_baseline(reader, binding_baseline)
        report = reader.backup_identity(destination, reader.description['storage_id'],
                                        progress=lambda text: print(text, flush=True))
        require(report.get('complete') is True, 'Identity backup is incomplete')
        sync_directory(destination.parent)
        fresh = {**binding_baseline, 'identity_sha256': report['backups'],
                 'source': 'known-device-stock-recovery-finalization',
                 'predecessor_sha256': digest(args.baseline), 'identity_decoded': False,
                 'normal_os_verified': False, 'stock_boot_operator_confirmed': True}
        compare_baseline(reader, fresh)
        retained = {name: reader.hash(name) for name in ('boot', 'odmdtbo')}
        require(retained == release['retained_stock'], 'Observed boot/odmdtbo differ from reviewed stock images')
        record = {'schema': 1, 'purpose': 'private-stock-recovery-finalization', 'complete': False,
                  'device': reader.description, 'identity_sha256': report['backups'],
                  'retained': retained, 'recovery_sha256': args.recovery_sha256,
                  'stock_manifest_sha256': args.stock_manifest_sha256,
                  'loader_sha256': args.loader_sha256, 'board_data_sha256': args.preloader_sha256,
                  'predecessor_sha256': fresh['predecessor_sha256'], 'write': 'pending',
                  'stock_boot_operator_confirmed': True, 'boot_requested': args.boot_after_capture,
                  'boot_acknowledged': False, 'usb_cleanup_verified': False}
        save_json(destination / 'journal.json', record)
        binding = {**reader.description, 'model': MODEL, 'model_verified': True,
                   'identity_sha256': fingerprint(report['backups'])}
        return writer_factory(mtk, revision, release=release, bundle=bundle, binding=binding)

    factory = lambda checkout: ExactUsbBackend(checkout, preloader=args.preloader,
                                               preloader_sha256=args.preloader_sha256)
    try:
        with session(args.checkout, args.loader, args.loader_sha256, args.lock_dir, None, factory,
                     candidate_provider=choose, device_factory=device_factory,
                     boot_after_capture=args.boot_after_capture) as device:
            with device:
                record['write'] = 'started'
                save_json(destination / 'journal.json', record)
                device.write('recovery', bundle / release['images']['recovery']['file'])
                require(device.hash('recovery') == args.recovery_sha256, 'Recovery readback mismatch')
                for name, checksum in record['retained'].items():
                    require(device.hash(name) == checksum, f'Retained {name} changed')
                compare_baseline(device, fresh)
                record['write'] = 'verified'
                record['identity_verified_after'] = True
                record['retained_verified_after'] = True
                save_json(destination / 'journal.json', record)
        record.update(complete=True, usb_cleanup_verified=True, boot_acknowledged=args.boot_after_capture)
        save_json(destination / 'journal.json', record)
        save_json(destination / 'baseline.json', fresh)
        print('Original recovery readback and retained boot/calibration verified; new baseline saved. No userdata operations were requested.')
        if args.boot_after_capture:
            print('DA exit acknowledged. Hold Power if off; subsequent Android startup remains unverified.')
    except BaseException as error:
        if record is not None:
            record['complete'] = False
            record['stopped'] = type(error).__name__
            save_json(destination / 'journal.json', record)
        raise


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('baseline', 'stock-manifest', 'recovery', 'checkout', 'loader', 'preloader', 'backup-dir'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('stock-manifest-sha256', 'recovery-sha256', 'confirm-cid-sha256', 'loader-sha256', 'preloader-sha256', 'ports'):
        parser.add_argument('--' + name, required=True)
    for name in ('allow-private-flash', 'stock-boot-confirmed', 'boot-after-capture', 'check-only'):
        parser.add_argument('--' + name, action='store_true')
    parser.add_argument('--bus', type=int, required=True)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--lock-dir', type=Path, default=Path.home() / '.local/state/couch-installer/locks')
    try:
        run(parser.parse_args(argv))
        return 0
    except (InstallError, OSError, ValueError, KeyError, TypeError) as error:
        print(f'Finalization stopped: {error}', file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print('Interrupted. Retain journal and originals; no automatic retry or reset.', file=sys.stderr)
        return 130


if __name__ == '__main__':
    sys.exit(main())
