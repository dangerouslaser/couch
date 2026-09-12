"""Unit tests for the factory firmware restore pin and change-detection helpers.

No network or device access. The pinned archive is a Sanytron factory
binpackage whose members are direct partition images.
"""
import hashlib
import io
import json
import tempfile
import unittest
from pathlib import Path
import zipfile

import factory_firmware_pin as firmware_restore


def _zip_with(members):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        for name, data in members.items():
            archive.writestr(name, data)
    return buffer.getvalue()


class FirmwareRestorePinTest(unittest.TestCase):
    def test_repo_pin_is_internally_consistent(self):
        pin = firmware_restore.load()
        self.assertEqual(pin['purpose'], 'restore-factory-firmware')
        self.assertFalse(pin['redistribution_authorized'])
        self.assertFalse(pin['preloader']['flashed'])
        self.assertIn('preloader', pin['preserved'])
        self.assertEqual(pin['userdata']['filesystem'], 'f2fs')
        self.assertFalse(pin['userdata']['reproducible_hash'])

    def test_drive_source_points_at_the_factory_archive(self):
        pin = firmware_restore.load()
        self.assertIn('drive.google.com', pin['source']['drive_url'])
        self.assertEqual(pin['source']['drive_file_id'], '1jOGJGGb-KPJJ8Ct4HqUtb1d-rKdEmKm9')
        self.assertEqual(pin['archive']['version'], 'V1.0.4_20260702')
        self.assertEqual(pin['source']['member_root'], 'binpackage/')

    def test_restore_images_cover_the_os_and_boot_chain(self):
        pin = firmware_restore.load()
        for name in ('system', 'vendor', 'boot', 'recovery', 'lk', 'lk2', 'cache', 'userdata'):
            self.assertIn(name, pin['restore_images'])
        self.assertNotIn('preloader', pin['restore_images'])

    def test_preloader_must_be_preserved(self):
        pin = json.loads(json.dumps(firmware_restore.load()))
        pin['preloader']['flashed'] = True
        with self.assertRaises(Exception):
            firmware_restore.load(pin)

    def test_missing_core_source_member_is_rejected(self):
        pin = json.loads(json.dumps(firmware_restore.load()))
        del pin['source_members']['system']
        with self.assertRaises(Exception):
            firmware_restore.load(pin)

    def test_verify_archive_checks_size_and_hash(self):
        data = b'factory archive bytes'
        with tempfile.TemporaryDirectory() as root:
            ota = Path(root) / 'fw.zip'
            ota.write_bytes(data)
            pin = json.loads(json.dumps(firmware_restore.load()))
            pin['archive']['size'] = len(data)
            pin['archive']['sha256'] = hashlib.sha256(data).hexdigest()
            self.assertEqual(firmware_restore.verify_archive(ota, pin), pin['archive']['sha256'])
            pin['archive']['sha256'] = '1' * 64
            with self.assertRaises(Exception):
                firmware_restore.verify_archive(ota, pin)

    def _fixture_pin_and_zip(self):
        boot = b'ANDROID!' + b'\x00' * 120
        system = b'\x00' * 2048
        vendor = b'\x00' * 1024
        scatter = b'- partition_name: system\n'
        preloader = b'MMM\x00' + b'\x00' * 60
        members = {'binpackage/boot.img': boot, 'binpackage/system.img': system,
                   'binpackage/vendor.img': vendor, 'binpackage/MT6580_Android_scatter.txt': scatter,
                   'binpackage/preloader_x15cm_s90_kr.bin': preloader}
        archive = _zip_with(members)

        def entry(data):
            return {'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        pin = json.loads(json.dumps(firmware_restore.load()))
        pin['archive']['size'] = len(archive)
        pin['archive']['sha256'] = hashlib.sha256(archive).hexdigest()
        pin['scatter'] = {'member': 'binpackage/MT6580_Android_scatter.txt', **entry(scatter)}
        pin['preloader'] = {'member': 'binpackage/preloader_x15cm_s90_kr.bin', 'flashed': False, **entry(preloader)}
        pin['source_members'] = {
            'boot': {'member': 'binpackage/boot.img', **entry(boot)},
            'system': {'member': 'binpackage/system.img', **entry(system)},
            'vendor': {'member': 'binpackage/vendor.img', **entry(vendor)},
        }
        return pin, archive

    def test_derive_reports_member_hashes(self):
        pin, archive = self._fixture_pin_and_zip()
        with tempfile.TemporaryDirectory() as root:
            ota = Path(root) / 'fw.zip'
            ota.write_bytes(archive)
            derived = firmware_restore.derive(ota, pin=pin)
            self.assertEqual(derived['source_members']['boot']['sha256'],
                             pin['source_members']['boot']['sha256'])
            self.assertEqual(derived['preloader']['sha256'], pin['preloader']['sha256'])
            firmware_restore.check(ota, pin)

    def test_check_detects_a_drifted_member(self):
        pin, archive = self._fixture_pin_and_zip()
        pin['source_members']['boot']['sha256'] = '0' * 64
        with tempfile.TemporaryDirectory() as root:
            ota = Path(root) / 'fw.zip'
            ota.write_bytes(archive)
            with self.assertRaises(Exception):
                firmware_restore.check(ota, pin)


if __name__ == '__main__':
    unittest.main()
