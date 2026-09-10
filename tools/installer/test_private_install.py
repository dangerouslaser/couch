from contextlib import redirect_stdout
import hashlib
import io
import json
from pathlib import Path
from types import SimpleNamespace as NS
import tempfile
import unittest
from unittest.mock import patch

from couch_install import IDENTITY_PARTITIONS, InstallError, load_release
from make_demo import create
import private_install


class PrivateInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        create(self.root)
        self.manifest = self.root/'release/manifest.json'
        release, _ = load_release(self.manifest)
        release.update(private_candidate=True, installable=False)
        self.manifest.write_text(json.dumps(release))
        cid = '1234567890abcdef1234567890abcdef'
        baseline = dict(schema=1, cid=cid, partitions=release['partitions'],
                        capacity=max(r['offset']+r['size'] for r in release['partitions'].values()),
                        identity_sha256={name:'a'*64 for name in IDENTITY_PARTITIONS})
        self.baseline = self.root/'baseline.json'
        self.baseline.write_text(json.dumps(baseline))
        self.args = NS(allow_private_flash=True, manifest=self.manifest, baseline=self.baseline,
                       confirm_cid_sha256=hashlib.sha256(bytes.fromhex(cid)).hexdigest(),
                       bus=1, ports='2.3', timeout=120, checkout=self.root,
                       loader=self.manifest, loader_sha256='a'*64, preloader=self.manifest,
                       preloader_sha256='b'*64, check_only=True)

    def test_check_only_checks_files_without_starting_usb(self):
        with patch('private_install.source_pin') as source, patch('private_install.loader_bytes') as loader:
            with redirect_stdout(io.StringIO()):
                private_install.run(self.args, session=lambda *a, **kw:self.fail('USB touched'))
            source.assert_called_once()
            self.assertEqual(loader.call_count,2)

    def test_refuses_wrong_target_and_public_manifest_before_usb(self):
        with patch('private_install.source_pin') as source:
            self.args.confirm_cid_sha256 = '0'*64
            with self.assertRaisesRegex(InstallError,'CID confirmation'):
                private_install.validate_inputs(self.args)
            source.assert_not_called()
        self.setUp()
        release=json.loads(self.manifest.read_text());release['installable']=True
        self.manifest.write_text(json.dumps(release))
        with self.assertRaisesRegex(InstallError,'private candidate'):
            private_install.validate_inputs(self.args)

    def test_requires_explicit_flash_flag_and_complete_identity(self):
        self.args.allow_private_flash=False
        with self.assertRaisesRegex(InstallError,'allow-private-flash'):
            private_install.validate_inputs(self.args)
        self.args.allow_private_flash=True
        baseline=json.loads(self.baseline.read_text());baseline.pop('identity_sha256')
        self.baseline.write_text(json.dumps(baseline))
        with self.assertRaisesRegex(InstallError,'complete independently'):
            private_install.validate_inputs(self.args)

    def test_rejects_bad_late_image_before_any_usb(self):
        image=self.root/'release/boot.img'
        image.write_bytes(b'damaged')
        with patch('private_install.source_pin') as source:
            with self.assertRaises(InstallError):
                private_install.validate_inputs(self.args)
            source.assert_not_called()
