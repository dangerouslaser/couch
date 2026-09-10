from contextlib import contextmanager, redirect_stdout
import io
import json
from unittest.mock import patch
import unittest

import test_capture_stock_identity as fixtures
from couch_install import InstallError, digest, read_json
from finalize_stock_recovery import run


class FinalizeTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.StockCaptureTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.args = self.fixture.args
        self.reader = self.fixture.reader
        original_hash = self.reader.hash
        self.reader.hash = lambda name: 'b'*64 if name == 'odmdtbo' else original_hash(name)
        profile = read_json(self.args.stock_manifest)
        profile['images'] = {name: {'sha256': self.reader.hash(name)} for name in ('boot', 'odmdtbo')}
        self.args.stock_manifest.write_text(json.dumps(profile))
        self.args.stock_manifest_sha256 = digest(self.args.stock_manifest)
        self.args.allow_private_flash = self.args.stock_boot_confirmed = True
        self.args.boot_after_capture = True
        self.args.recovery = self.fixture.fixture.root / 'recovery.img'
        self.args.recovery.write_bytes(b'ANDROID!' + b'\0' * (self.reader.description['partitions']['recovery']['size'] - 8))
        self.args.recovery_sha256 = digest(self.args.recovery)
        self.writes = []
        self.exits = 0
        self.fail_readback = self.fail_cleanup = False

    @contextmanager
    def session(self, *args, **kwargs):
        device = kwargs['device_factory'](None, None)
        yield device
        if kwargs['boot_after_capture']:
            self.exits += 1
        if self.fail_cleanup:
            raise InstallError('cleanup failed')

    def writer(self, mtk, revision, **kwargs):
        self.assertEqual(set(kwargs['release']['images']), {'recovery'})
        self.assertTrue((self.args.backup_dir / 'journal.json').exists())
        parent = self
        class Writer:
            description = parent.reader.description
            def __enter__(self):
                return self
            def __exit__(self, *args):
                pass
            def write(self, name, path):
                parent.assertEqual(read_json(parent.args.backup_dir / 'journal.json')['write'], 'started')
                parent.writes.append(name)
            def hash(self, name):
                parent.assertNotEqual(name, 'userdata')
                if name == 'recovery':
                    return '0' * 64 if parent.fail_readback else parent.args.recovery_sha256
                return parent.reader.hash(name)
        return Writer()

    def execute(self):
        with patch('finalize_stock_recovery.source_pin', return_value={}), redirect_stdout(io.StringIO()):
            run(self.args, session=self.session, reader_factory=lambda *args: self.reader, writer_factory=self.writer)

    def test_only_recovery_written_after_fresh_capture_and_retained_images_verified(self):
        self.execute()
        self.assertEqual(self.writes, ['recovery'])
        self.assertEqual(self.exits, 1)
        self.assertTrue(read_json(self.args.backup_dir / 'journal.json')['complete'])
        self.assertTrue((self.args.backup_dir / 'baseline.json').exists())
        self.assertEqual(self.args.baseline.read_bytes(), self.fixture.original)

    def test_readback_failure_never_exits_or_publishes_baseline(self):
        self.fail_readback = True
        with self.assertRaisesRegex(InstallError, 'readback mismatch'):
            self.execute()
        self.assertEqual(self.exits, 0)
        self.assertFalse((self.args.backup_dir / 'baseline.json').exists())
        self.assertFalse(read_json(self.args.backup_dir / 'journal.json')['complete'])

    def test_cleanup_failure_prevents_baseline_publication(self):
        self.fail_cleanup = True
        with self.assertRaisesRegex(InstallError, 'cleanup failed'):
            self.execute()
        self.assertFalse((self.args.backup_dir / 'baseline.json').exists())

    def test_confirmation_cid_and_image_fail_before_write(self):
        for field, wrong in [('stock_boot_confirmed', False), ('allow_private_flash', False),
                             ('confirm_cid_sha256', '0'*64), ('recovery_sha256', '0'*64)]:
            previous = getattr(self.args, field)
            setattr(self.args, field, wrong)
            with self.assertRaises(InstallError):
                self.execute()
            setattr(self.args, field, previous)
        self.assertEqual(self.writes, [])
        self.assertFalse(self.args.backup_dir.exists())

    def test_unexpected_stock_boot_stops_before_writer(self):
        original_hash = self.reader.hash
        self.reader.hash = lambda name: '0'*64 if name == 'boot' else original_hash(name)
        with self.assertRaisesRegex(InstallError, 'differ from reviewed stock'):
            self.execute()
        self.assertEqual(self.writes, [])
        self.assertEqual(self.exits, 0)

    def test_retained_boot_change_after_write_stops_exit(self):
        original_hash = self.reader.hash
        self.reader.hash = lambda name: '0'*64 if name == 'boot' and self.writes else original_hash(name)
        with self.assertRaisesRegex(InstallError, 'Retained boot changed'):
            self.execute()
        self.assertEqual(self.writes, ['recovery'])
        self.assertEqual(self.exits, 0)
        self.assertFalse((self.args.backup_dir / 'baseline.json').exists())

    def test_check_only_no_session(self):
        self.args.check_only = True
        self.execute()
        self.assertEqual(self.writes, [])
        self.assertEqual(self.exits, 0)
        self.assertFalse(self.args.backup_dir.exists())

    def test_changed_calibration_readback_stops_before_writer(self):
        with patch.object(self.reader, 'hash', return_value='0'*64):
            with self.assertRaisesRegex(InstallError, 'Independent identity readback mismatch'):
                self.execute()
        self.assertEqual(self.writes, [])
        self.assertEqual(self.exits, 0)


if __name__ == '__main__':
    unittest.main()
