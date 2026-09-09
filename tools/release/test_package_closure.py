import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from package_closure import IMAGE, PREFIX, inventory, prepare, verify


class PackageClosureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def fixture(self):
        for name in ('packages', 'indexes', 'keys'):
            (self.root / name).mkdir()
        (self.root / 'packages/musl-1.2.5-r11.apk').write_bytes(b'fixture-package')
        (self.root / 'indexes/APKINDEX.test.tar.gz').write_bytes(b'fixture-index')
        (self.root / 'keys/alpine.pub').write_bytes(b'fixture-public-key')
        (self.root / 'package-urls.txt').write_text(PREFIX + 'main/armv7/musl-1.2.5-r11.apk\n')
        manifest = inventory(self.root, ['musl'], IMAGE)
        (self.root / 'closure.json').write_text(json.dumps(manifest))
        return manifest

    def test_inventory_is_stable_and_hash_verification_detects_changes(self):
        manifest = self.fixture()
        self.assertEqual(verify(self.root), manifest)
        self.assertFalse(manifest['installable'])
        (self.root / 'packages/musl-1.2.5-r11.apk').write_bytes(b'tampered')
        with self.assertRaisesRegex(ValueError, 'changed'):
            verify(self.root)

    def test_extra_private_file_is_not_silently_ignored(self):
        self.fixture()
        (self.root / 'developer.key').write_text('private')
        with self.assertRaisesRegex(ValueError, 'unexpected'):
            verify(self.root)

    def test_rejects_symlinks_and_unexpected_sources(self):
        self.fixture()
        (self.root / 'keys/link').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(ValueError, 'Symlinks'):
            verify(self.root)
        (self.root / 'keys/link').unlink()
        (self.root / 'package-urls.txt').write_text('https://example.org/musl.apk\n')
        with self.assertRaisesRegex(ValueError, 'source URL'):
            inventory(self.root, ['musl'], IMAGE)

    def test_missing_package_and_inventory_metadata_are_rejected(self):
        manifest = self.fixture()
        manifest['packages'][0]['url'] = 'https://example.org/musl.apk'
        (self.root / 'closure.json').write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, 'metadata'):
            verify(self.root)
        (self.root / 'packages/musl-1.2.5-r11.apk').unlink()
        with self.assertRaisesRegex(ValueError, 'package set'):
            inventory(self.root, ['musl'], IMAGE)

    def test_x86_tool_closure_is_explicit_and_cannot_masquerade_as_arm(self):
        self.fixture()
        path = self.root / 'package-urls.txt'
        path.write_text(path.read_text().replace('/armv7/', '/x86_64/'))
        manifest = inventory(self.root, ['musl'], IMAGE, 'x86_64')
        self.assertEqual(manifest['architecture'], 'x86_64')
        with self.assertRaisesRegex(ValueError, 'source path'):
            inventory(self.root, ['musl'], IMAGE)
        with self.assertRaisesRegex(ValueError, 'architecture'):
            inventory(self.root, ['musl'], IMAGE, 'unknown')

    def test_prepare_rejects_unpinned_builder_options_and_existing_output(self):
        with patch('package_closure.subprocess.run') as run:
            with self.assertRaises(ValueError):
                prepare(self.root / 'new', 'alpine:latest', ['musl'])
            with self.assertRaises(ValueError):
                prepare(self.root / 'new', IMAGE, ['--allow-untrusted'])
            with self.assertRaises(FileExistsError):
                prepare(self.root, IMAGE, ['musl'])
            run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
