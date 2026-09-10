from contextlib import contextmanager, redirect_stdout
import hashlib
import io
import json
from unittest.mock import patch
import unittest

from capture_stock_identity import recapture
from capture_readonly import capture
from couch_install import IDENTITY_PARTITIONS, MODEL, InstallError, digest, read_json
import test_capture_readonly as fixtures


class StockCaptureTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.CaptureTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.args = self.fixture.args
        self.reader = self.fixture.reader
        baseline = {'schema': 1, 'capacity': self.reader.capacity,
                    'partitions': self.reader.description['partitions'],
                    'cid': '000000007b00000000000000c8010000',
                    'identity_sha256': {name: 'a' * 64 for name in IDENTITY_PARTITIONS}}
        self.args.baseline = self.fixture.root / 'old.json'
        self.args.baseline.write_text(json.dumps(baseline))
        self.original = self.args.baseline.read_bytes()
        self.args.confirm_cid_sha256 = hashlib.sha256(bytes.fromhex(baseline['cid'])).hexdigest()
        self.args.stock_manifest = self.fixture.root / 'stock.json'
        self.args.stock_manifest.write_text(json.dumps({'schema': 1, 'model': MODEL,
            'purpose': 'stock-baseline', 'private_candidate': True, 'installable': False,
            'partitions': baseline['partitions']}))
        self.args.stock_manifest_sha256 = digest(self.args.stock_manifest)
        self.sessions = 0
        self.cleanup_fails = False
        self.exits = 0

    def perform_capture(self, args, **capture_options):
        @contextmanager
        def session(*unused, **kwargs):
            self.sessions += 1
            yield self.reader
            if kwargs['boot_after_capture']:
                self.exits += 1
            if self.cleanup_fails:
                raise InstallError('USB cleanup failed')
        with patch('capture_readonly.source_pin', return_value={}), redirect_stdout(io.StringIO()):
            capture(args, session=session, **capture_options)

    def test_mutable_identity_recaptured_old_baseline_preserved(self):
        with redirect_stdout(io.StringIO()):
            recapture(self.args, capture_fn=self.perform_capture)
        fresh = read_json(self.args.backup_dir / 'baseline.json')
        self.assertEqual(self.args.baseline.read_bytes(), self.original)
        self.assertEqual(fresh['cid'], json.loads(self.original)['cid'])
        self.assertEqual(fresh['identity_sha256'], {name: self.reader.hash(name) for name in IDENTITY_PARTITIONS})
        self.assertEqual(fresh['predecessor_sha256'], hashlib.sha256(self.original).hexdigest())
        self.assertFalse(fresh['identity_decoded'])

    def test_explicit_exit_only_after_successful_independent_capture(self):
        self.args.boot_after_capture = True
        with patch.object(self.reader, 'hash', return_value='0' * 64):
            with self.assertRaises(InstallError):
                recapture(self.args, capture_fn=self.perform_capture)
        self.assertEqual(self.exits, 0)
        self.args.backup_dir = self.fixture.root / 'successful-exit'
        with redirect_stdout(io.StringIO()):
            recapture(self.args, capture_fn=self.perform_capture)
        self.assertEqual(self.exits, 1)
        fresh = read_json(self.args.backup_dir / 'baseline.json')
        self.assertTrue(fresh['boot_acknowledged'])
        self.assertFalse(fresh['normal_os_verified'])

    def test_wrong_confirmation_and_profile_pin_fail_before_session(self):
        for field in ('confirm_cid_sha256', 'stock_manifest_sha256'):
            before = getattr(self.args, field)
            setattr(self.args, field, '0' * 64)
            with self.assertRaises(InstallError):
                recapture(self.args, capture_fn=self.perform_capture)
            setattr(self.args, field, before)
        self.assertEqual(self.sessions, 0)
        self.assertFalse(self.args.backup_dir.exists())

    def test_wrong_observed_cid_and_cleanup_failure_never_publish_baseline(self):
        self.reader.description['runtime_cid_sha256'] = '0' * 64
        with self.assertRaisesRegex(InstallError, 'CID differs'):
            recapture(self.args, capture_fn=self.perform_capture)
        self.assertFalse(self.args.backup_dir.exists())
        self.reader.description['runtime_cid_sha256'] = self.args.confirm_cid_sha256
        self.cleanup_fails = True
        with self.assertRaisesRegex(InstallError, 'cleanup failed'):
            recapture(self.args, capture_fn=self.perform_capture)
        self.assertFalse((self.args.backup_dir / 'baseline.json').exists())

    def test_existing_destination_and_changed_profile_layout_are_rejected(self):
        self.args.backup_dir.mkdir()
        with self.assertRaisesRegex(InstallError, 'new stock identity destination'):
            recapture(self.args, capture_fn=self.perform_capture)
        self.args.backup_dir.rmdir()
        profile = read_json(self.args.stock_manifest)
        profile['partitions']['userdata']['size'] -= 512
        self.args.stock_manifest.write_text(json.dumps(profile))
        self.args.stock_manifest_sha256 = digest(self.args.stock_manifest)
        with self.assertRaises(InstallError):
            recapture(self.args, capture_fn=self.perform_capture)
        self.assertEqual(self.sessions, 0)

    def test_check_only_does_not_open_usb_or_emit_baseline(self):
        self.args.check_only = True
        with redirect_stdout(io.StringIO()):
            recapture(self.args, capture_fn=self.perform_capture)
        self.assertEqual(self.sessions, 0)
        self.assertFalse(self.args.backup_dir.exists())

    def test_independent_reread_failure_never_emits_baseline(self):
        with patch.object(self.reader, 'hash', return_value='0' * 64):
            with self.assertRaisesRegex(InstallError, 'Independent identity readback mismatch'):
                recapture(self.args, capture_fn=self.perform_capture)
        self.assertFalse((self.args.backup_dir / 'baseline.json').exists())


if __name__ == '__main__':
    unittest.main()
