"""Unit tests for the firmware restore pin and change-detection helpers.

No network, device, brotli or debugfs access: the formatter derivation is
best-effort and is not required for these tests.
"""
import hashlib
import io
import json
import tempfile
import unittest
from pathlib import Path
import zipfile

import firmware_restore


def _zip_with(members):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        for name, data in members.items():
            archive.writestr(name, data)
    return buffer.getvalue()


class FirmwareRestorePinTest(unittest.TestCase):
    def test_repo_pin_is_consistent_with_official_runtime(self):
        pin = firmware_restore.load()
        self.assertEqual(pin['purpose'], 'restore-original-android')
        self.assertFalse(pin['redistribution_authorized'])
        self.assertEqual(pin['userdata']['filesystem'], 'f2fs')
        self.assertFalse(pin['userdata']['reproducible_hash'])

    def test_drive_and_cdn_sources_are_recorded(self):
        pin = firmware_restore.load()
        self.assertIn('drive.google.com', pin['source']['drive_url'])
        self.assertEqual(pin['source']['drive_file_id'], '1jOGJGGb-KPJJ8Ct4HqUtb1d-rKdEmKm9')
        self.assertTrue(pin['source']['cdn_url'].endswith('.zip'))

    def test_archive_mismatch_with_official_pin_is_rejected(self):
        pin = firmware_restore.load()
        official = json.loads(firmware_restore.OFFICIAL.read_text())
        tampered = json.loads(json.dumps(pin))
        tampered['archive']['sha256'] = '0' * 64
        with self.assertRaises(Exception):
            firmware_restore.load(tampered, official)

    def test_verify_archive_checks_size_and_hash(self):
        data = b'firmware archive bytes'
        with tempfile.TemporaryDirectory() as root:
            ota = Path(root) / 'ota.zip'
            ota.write_bytes(data)
            pin = json.loads(json.dumps(firmware_restore.load()))
            pin['archive']['size'] = len(data)
            pin['archive']['sha256'] = hashlib.sha256(data).hexdigest()
            self.assertEqual(firmware_restore.verify_archive(ota, pin), pin['archive']['sha256'])
            pin['archive']['sha256'] = '1' * 64
            with self.assertRaises(Exception):
                firmware_restore.verify_archive(ota, pin)

    def test_derive_reports_boot_and_overlay_member_hashes(self):
        boot = b'ANDROID!' + b'\x00' * 120
        overlay = b'\xd0\x0d\xfe\xed' + b'\x00' * 60
        archive = _zip_with({'boot.img': boot, 'odmdtbo.img': overlay,
                             'system.new.dat.br': b'x', 'unused': b'y'})
        with tempfile.TemporaryDirectory() as root:
            ota = Path(root) / 'ota.zip'
            ota.write_bytes(archive)
            derived = firmware_restore.derive(ota, with_formatter=False)
            self.assertEqual(derived['images']['boot']['sha256'], hashlib.sha256(boot).hexdigest())
            self.assertEqual(derived['images']['odmdtbo']['size'], len(overlay))

    def test_check_detects_drifted_boot_image(self):
        boot = b'ANDROID!' + b'\x00' * 120
        overlay = b'\xd0\x0d\xfe\xed' + b'\x00' * 60
        archive = _zip_with({'boot.img': boot, 'odmdtbo.img': overlay})
        with tempfile.TemporaryDirectory() as root:
            ota = Path(root) / 'ota.zip'
            ota.write_bytes(archive)
            pin = json.loads(json.dumps(firmware_restore.load()))
            pin['archive']['size'] = len(archive)
            pin['archive']['sha256'] = hashlib.sha256(archive).hexdigest()
            # The real pinned boot hash will not match this fixture archive.
            with self.assertRaises(Exception):
                firmware_restore.check(ota, pin)


if __name__ == '__main__':
    unittest.main()
