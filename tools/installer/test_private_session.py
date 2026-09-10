"""Private CLI/transaction integration on regular-file fixtures; never opens USB."""
from contextlib import contextmanager, redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import couch_install as core
import make_demo
import private_install as private


class PrivateSessionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        make_demo.create(self.root)
        manifest = self.root/'release/manifest.json'
        release = core.read_json(manifest)
        release.update(private_candidate=True, installable=False)
        core.save_json(manifest, release)
        self.device = core.FileDevice(self.root/'device')
        cid = '1234567890abcdef1234567890abcdef'
        cid_hash = hashlib.sha256(bytes.fromhex(cid)).hexdigest()
        self.baseline = {'schema':1, 'cid':cid,
            'capacity':max(r['offset']+r['size'] for r in release['partitions'].values()),
            'partitions':release['partitions'],
            'identity_sha256':{name:self.device.hash(name) for name in core.IDENTITY_PARTITIONS}}
        core.save_json(self.root/'baseline.json', self.baseline)
        self.loader = self.root/'loader.bin'
        self.loader.write_bytes(b'private loader fixture')
        self.args = SimpleNamespace(allow_private_flash=True, manifest=manifest,
            baseline=self.root/'baseline.json', confirm_cid_sha256=cid_hash,
            ports='2.1', bus=1, timeout=1, checkout=self.root/'checkout', loader=self.loader,
            loader_sha256=core.digest(self.loader), preloader=self.loader,
            preloader_sha256=core.digest(self.loader), backup_dir=self.root/'originals',
            lock_dir=self.root/'locks', resume=False, restore=False, boot_after_install=False,
            check_only=False)
        self.events = []
        self.description = dict(self.device.description,
            capacity=self.baseline['capacity'], cid_encoding='mt6580-legacy-le32-registers',
            runtime_cid_sha256=cid_hash)
        self.output = io.StringIO()

    def run_private(self):
        test = self
        class Reader:
            def __init__(self, mtk, revision):
                self.description = dict(test.description)
            def hash(self, name):
                return test.device.hash(name)
        class Writer(core.FileDevice):
            def __init__(self, mtk, revision, *, release, bundle, binding):
                super().__init__(test.root/'device')
                self.description = dict(test.description, model_verified=True,
                    transport='fake-private-writer', identity_sha256=binding['identity_sha256'])
                test.events.append('writer-constructed')
                for name, image in release['images'].items():
                    core.require(core.digest(bundle/image['file']) == image['sha256'], 'Verified source mismatch')
            def __enter__(self):
                return self
            def __exit__(self, *args):
                test.events.append('writer-closed')
            def write(self, name, source):
                test.events.append(('write', name))
                super().write(name, source)
        @contextmanager
        def session(*args, device_factory, boot_after_capture, **kwargs):
            test.events.append('session-open')
            try:
                yield device_factory(object(), 'fixture revision')
                if boot_after_capture:
                    test.events.append('boot-requested')
            finally:
                test.events.append('session-closed')
        with patch.object(private, 'source_pin', return_value={}), \
                patch.object(private, 'ConnectedMtkReader', Reader), \
                patch('mtk_writer.ConnectedMtkWriter', Writer), redirect_stdout(self.output):
            private.run(self.args, session=session)

    def test_install_and_restore_use_same_binding_and_restore_survives_missing_candidate_images(self):
        originals = {name:self.device.hash(name) for name in core.REQUIRED_IMAGES}
        self.run_private()
        self.assertEqual([e for e in self.events if isinstance(e,tuple)],
                         [('write','recovery'),('write','userdata'),('write','boot')])
        self.assertNotIn('boot-requested', self.events)
        for image in (self.root/'release').glob('*.img'):
            image.unlink()
        self.args.restore = True
        self.args.boot_after_install = True
        self.events.clear()
        self.run_private()
        self.assertEqual([e for e in self.events if isinstance(e,tuple)],
                         [('write','userdata'),('write','boot'),('write','recovery')])
        self.assertEqual(self.events[-3:], ['writer-closed','boot-requested','session-closed'])
        for name, checksum in originals.items():
            self.assertEqual(self.device.hash(name), checksum)
        self.assertIn('Original partition restore readback completed', self.output.getvalue())

    def test_corrupted_originals_prevent_restore_writes_and_boot(self):
        self.run_private()
        (self.args.backup_dir/'boot.img').write_bytes(bytes(4096))
        self.args.restore = True
        self.args.boot_after_install = True
        self.events.clear()
        with self.assertRaises(core.InstallError):
            self.run_private()
        self.assertFalse(any(isinstance(e,tuple) for e in self.events))
        self.assertNotIn('boot-requested', self.events)
        self.assertEqual(self.events[-1], 'session-closed')

    def test_changed_calibration_or_cid_prevents_writer_construction(self):
        self.description['runtime_cid_sha256'] = '0'*64
        with self.assertRaisesRegex(core.InstallError, 'CID'):
            self.run_private()
        self.assertNotIn('writer-constructed', self.events)
        self.description['runtime_cid_sha256'] = self.args.confirm_cid_sha256
        self.device.partition('nvram').write_bytes(bytes(4096))
        self.events.clear()
        with self.assertRaisesRegex(core.InstallError, 'baseline differs'):
            self.run_private()
        self.assertNotIn('writer-constructed', self.events)

    def test_install_still_requires_candidate_images_and_private_opt_in_before_session(self):
        self.args.allow_private_flash = False
        with self.assertRaisesRegex(core.InstallError, 'allow-private-flash'):
            self.run_private()
        self.assertEqual(self.events, [])
        self.args.allow_private_flash = True
        (self.root/'release/boot.img').unlink()
        with self.assertRaises(OSError):
            self.run_private()
        self.assertEqual(self.events, [])

    def test_metadata_only_release_validation_still_rejects_unsafe_sources_and_prohibited_targets(self):
        release = core.read_json(self.args.manifest)
        for bad in ({**release, 'images':{**release['images'], 'lk':release['images']['boot']}},
                    {**release, 'images':{**release['images'], 'boot':{'file':'../boot.img','sha256':'0'*64}}},
                    {**release, 'images':{**release['images'], 'boot':{'file':'boot.img','sha256':'bad'}}}):
            core.save_json(self.args.manifest, bad)
            with self.assertRaises(core.InstallError):
                core.load_release(self.args.manifest, verify_images=False)


if __name__ == '__main__':
    unittest.main()
