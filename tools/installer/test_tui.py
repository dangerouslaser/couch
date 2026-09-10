import argparse
import hashlib
import io
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import couch_tui as tui
import make_demo


class TuiTests(unittest.TestCase):
    def test_cancel_does_not_call_apply_and_success_streams_core_progress(self):
        class Adapter:
            called = False
            def plan(self):
                return tui.Plan('fixture', 'local release', ('nvram',), ('recovery',), Path('/private/backups'), True)
            def apply(self, plan, confirmation, progress):
                self.called = True
                progress.write('Verified recovery readback\n')
        cancelled = Adapter()
        output = io.StringIO()
        self.assertEqual(tui.Terminal(io.StringIO('1\nwrong\nq\n'), output).run(cancelled, True), 0)
        self.assertFalse(cancelled.called)
        accepted = Adapter()
        self.assertEqual(tui.Terminal(io.StringIO('1\nfixture\nq\n'), output).run(accepted, True), 0)
        self.assertTrue(accepted.called)
        self.assertIn('Verified recovery readback', output.getvalue())

    def test_real_policy_simulation_runs_only_after_explicit_target_confirmation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            make_demo.create(root)
            args = tui.parser().parse_args(['--simulation', '--manifest', str(root/'release/manifest.json'),
                '--device-dir', str(root/'device'), '--identity', str(root/'identity.json'), '--backup-dir', str(root/'originals')])
            output = io.StringIO()
            tui.Terminal(io.StringIO('1\nSIMULATED-HA100-001\nq\n'), output).run(tui.CoreAdapter(args), True)
            self.assertTrue((root/'originals/journal.json').exists())
            self.assertIn('No physical device', output.getvalue())

    def test_no_simulation_fallback_when_physical_install_is_unavailable(self):
        adapter = tui.CoreAdapter(tui.parser().parse_args([]))
        with self.assertRaisesRegex(tui.core.InstallError, 'not approved'):
            adapter.plan()
        self.assertIsNone(adapter.prepared)

    def test_no_terminal_fails_cleanly(self):
        with patch.object(tui.sys, 'platform', 'linux'), patch.object(tui.sys, 'stdin', io.StringIO()), \
                patch('builtins.open', side_effect=OSError), patch.object(tui.sys, 'stderr', io.StringIO()) as error:
            self.assertEqual(tui.main([]), 2)
            self.assertIn('interactive terminal', error.getvalue())


class BootstrapTests(unittest.TestCase):
    script = Path(__file__).with_name('bootstrap.sh')

    def test_no_approved_release_never_downloads(self):
        result = subprocess.run(['sh', str(self.script)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn('Nothing was downloaded', result.stderr)

    def verify(self, entries, corrupt=False):
        code = self.script.read_text().split("<<'COUCH_VERIFY_ARCHIVE'\n",1)[1].split('\nCOUCH_VERIFY_ARCHIVE',1)[0]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root/'release.tar.gz'
            with tarfile.open(archive, 'w:gz') as bundle:
                for name, kind in entries:
                    entry = tarfile.TarInfo(name)
                    if kind == 'link':
                        entry.type = tarfile.SYMTYPE
                        entry.linkname = '/etc/passwd'
                        bundle.addfile(entry)
                    else:
                        data = b'print("fixture")\n'
                        entry.size = len(data)
                        bundle.addfile(entry, io.BytesIO(data))
            checksum = '0'*64 if corrupt else hashlib.sha256(archive.read_bytes()).hexdigest()
            result = subprocess.run([sys.executable, '-', str(archive), checksum, str(root/'payload')], input=code, capture_output=True, text=True)
            return result, (root/'payload').exists()

    def test_verified_release_extracts_without_executing_it(self):
        result, exists = self.verify([('couch-installer/tools/installer/couch_tui.py', 'file')])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(exists)
        self.assertNotIn('fixture', result.stdout)

    def test_hash_paths_and_links_fail_before_extraction(self):
        for entries, corrupt in [([('couch-installer/tools/installer/couch_tui.py','file')],True),
                ([('../escape','file')],False), ([('couch-installer/link','link')],False)]:
            result, exists = self.verify(entries, corrupt)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(exists)


if __name__ == '__main__':
    unittest.main()
