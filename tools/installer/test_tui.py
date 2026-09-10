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



class PrivateTuiTests(unittest.TestCase):
    def setUp(self):
        import json
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.config = self.root/'trial.json'
        self.values = {key:str(self.root/key) for key in tui.PrivateAdapter.PATHS}
        self.values.update(loader_sha256='a'*64, preloader_sha256='b'*64,
            confirm_cid_sha256='c'*64, ports='2.1', bus=1, timeout=120)
        self.config.write_text(json.dumps(self.values))
        self.config.chmod(0o600)
        self.launches = []
        self.validations = []

    def validate(self, args):
        self.validations.append((args.restore, args.resume))
        return {'images':{'recovery':{},'userdata':{},'boot':{}}}, self.root, {}, (2,1)

    def launch(self, argv, **kwargs):
        self.launches.append((argv, kwargs))
        class Process:
            stdout = io.StringIO('Verified fixture readback\n')
            def wait(self, **kwargs):
                return 0
            def poll(self):
                return 0
        return Process()

    def adapter(self):
        return tui.PrivateAdapter(self.config, validate=self.validate, launch=self.launch)

    def test_private_plan_and_cancel_never_construct_child_or_usb_session(self):
        adapter = self.adapter()
        output = io.StringIO()
        tui.Terminal(io.StringIO('1\nwrong\nq\n'), output).run(adapter)
        self.assertEqual(self.validations, [(False,False)])
        self.assertEqual(self.launches, [])
        self.assertIn('No USB session is opened', output.getvalue())
        self.assertIn('Cancelled', output.getvalue())

    def test_exact_target_argv_restore_resume_and_no_shell(self):
        adapter = self.adapter()
        adapter.toggle_restore()
        adapter.toggle_resume()
        plan = adapter.plan()
        self.assertEqual(plan.writes, ('userdata','boot','recovery'))
        self.assertTrue(plan.resume)
        output = io.StringIO()
        adapter.apply(plan, 'c'*64, output)
        argv, kwargs = self.launches[0]
        self.assertEqual(argv[:3], [sys.executable, str(Path(tui.__file__).with_name('private_install.py')), '--allow-private-flash'])
        self.assertIn('--restore', argv)
        self.assertIn('--resume', argv)
        self.assertNotIn('--boot-after-install', argv)
        self.assertEqual(argv[argv.index('--confirm-cid-sha256')+1], 'c'*64)
        self.assertEqual(argv[argv.index('--backup-dir')+1], self.values['backup_dir'])
        self.assertFalse(kwargs.get('shell',False))
        self.assertEqual(kwargs['stdin'], subprocess.DEVNULL)
        self.assertTrue(kwargs['start_new_session'])
        self.assertIn('Verified fixture readback', output.getvalue())
        with self.assertRaises(tui.core.InstallError):
            adapter.apply(plan, 'c'*64, output)
        self.assertEqual(len(self.launches),1)

    def test_changing_operation_or_inexact_confirmation_requires_new_review(self):
        adapter = self.adapter()
        plan = adapter.plan()
        with self.assertRaises(tui.core.InstallError):
            adapter.apply(plan, 'wrong', io.StringIO())
        adapter.toggle_restore()
        with self.assertRaises(tui.core.InstallError):
            adapter.apply(plan, 'c'*64, io.StringIO())
        self.assertEqual(self.launches, [])

    def test_private_config_rejects_world_readable_files_unknown_fields_and_shelllike_boolean(self):
        import json
        self.config.chmod(0o644)
        with self.assertRaisesRegex(tui.core.InstallError, 'permissions'):
            self.adapter()
        self.config.chmod(0o600)
        for change in ({'password':'not-loggable'}, {'resume':'false'}):
            self.config.write_text(json.dumps({**self.values, **change}))
            with self.assertRaises(tui.core.InstallError) as raised:
                self.adapter()
            self.assertNotIn('not-loggable', str(raised.exception))

    def test_interrupt_stops_child_and_preserves_journal_message(self):
        signals = []
        class InterruptedOutput:
            def __iter__(self):
                raise KeyboardInterrupt
            def close(self):
                pass
        class Process:
            stdout = InterruptedOutput()
            def poll(self):
                return None
            def send_signal(self, value):
                signals.append(value)
            def wait(self, **kwargs):
                return 130
        adapter = tui.PrivateAdapter(self.config, validate=self.validate, launch=lambda *a,**kw:Process())
        output = io.StringIO()
        result = tui.Terminal(io.StringIO('1\n'+'c'*64+'\n'), output).run(adapter)
        self.assertEqual(result, 130)
        self.assertEqual(signals, [tui.signal.SIGINT])
        self.assertIn('do not delete recovery information', output.getvalue())
        self.assertNotIn('readback completed', output.getvalue())

    def test_nonzero_child_exit_never_reports_success(self):
        class Process:
            stdout = io.StringIO('Stopped: fixture readback mismatch\n')
            def wait(self, **kwargs):
                return 1
            def poll(self):
                return 1
        adapter = tui.PrivateAdapter(self.config, validate=self.validate, launch=lambda *a,**kw:Process())
        output = io.StringIO()
        tui.Terminal(io.StringIO('1\n'+'c'*64+'\nq\n'), output).run(adapter)
        self.assertIn('Keep the originals and journal',output.getvalue())
        self.assertNotIn('readback completed',output.getvalue())


class PhaseProgressTests(unittest.TestCase):
    def test_rate_eta_and_phase_reset_are_measured(self):
        output = io.StringIO()
        now = [0.0]
        display = tui.PhaseProgress(output, lambda: now[0])
        display.line('Backup: userdata 0/4194304 bytes (0%)')
        self.assertIn('ETA --:--:--', output.getvalue())
        now[0] = 2
        display.line('Backup: userdata 2097152/4194304 bytes (50%)')
        self.assertIn('1.00 MiB/s | elapsed 00:00:02 ETA 00:00:02', output.getvalue())
        now[0] = 3
        display.line('Hash readback: userdata 0/4194304 bytes (0%)')
        self.assertTrue(output.getvalue().endswith('0.00 MiB/s | elapsed 00:00:00 ETA --:--:--'))
        self.assertEqual(output.getvalue().count('\n'), 1)

    def test_rewind_resets_rate_and_unknown_logs_do_not_invent_progress(self):
        output = io.StringIO()
        now = [0.0]
        display = tui.PhaseProgress(output, lambda: now[0])
        display.line('Write: boot 50/100 bytes (50%)')
        now[0] = 5
        display.line('Write: boot 0/100 bytes (0%)')
        self.assertTrue(output.getvalue().endswith('elapsed 00:00:00 ETA --:--:--'))
        display.line('USB waiting')
        display.line('USB waiting again')
        self.assertEqual(output.getvalue().count('\n'), 1)
        display.line('Stopped: readback mismatch')
        self.assertTrue(output.getvalue().endswith('Stopped: readback mismatch\n'))
        display.line('Backup: userdata 0/0 bytes (0%)')
        display.finish()
        self.assertNotIn('nan', output.getvalue())

    def test_completion_and_nonadvancing_counters(self):
        output = io.StringIO()
        now = [0.0]
        display = tui.PhaseProgress(output, lambda: now[0])
        display.line('Verify image: boot 0/100 bytes (0%)')
        display.line('Verify image: boot 0/100 bytes (0%)')
        now[0] = 1
        display.line('Verify image: boot 100/100 bytes (100%)')
        self.assertTrue(output.getvalue().endswith('ETA 00:00:00\n'))
        before = output.getvalue()
        display.finish()
        self.assertEqual(output.getvalue(), before)


if __name__ == '__main__':
    unittest.main()
