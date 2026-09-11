import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import prepare


class AdmissionTests(unittest.TestCase):
    def test_build_reference_is_not_an_arbitrary_api_path(self):
        with patch.dict(os.environ, {'BINARY_RUN_ID':'../other', 'CONFIG_SHA256':'1'*64, 'LAUNCHER_SHA256':'2'*64}):
            with self.assertRaisesRegex(ValueError, 'run ID'):
                prepare.prepare(Path('.'), Path('.'), Path('unused'))

    def test_changed_descriptor_is_rejected_before_artifact_access(self):
        with patch.dict(os.environ, {'BINARY_RUN_ID':'123', 'CONFIG_SHA256':'1'*64, 'LAUNCHER_SHA256':'2'*64, 'CONFIG_BASE64':'e30='}), patch.object(prepare.subprocess, 'check_output', side_effect=[b'{"head_sha":"'+prepare.SOURCE.encode()+b'","conclusion":"success","name":"Build installer binaries"}', prepare.SOURCE+'\n']):
            with self.assertRaisesRegex(ValueError, 'descriptor hash'):
                prepare.prepare(Path('absent'), Path('absent'), Path('unused'))

    def test_failed_or_different_source_run_is_never_admitted(self):
        for source, conclusion in [('0'*40,'success'), (prepare.SOURCE,'failure')]:
            data = ('{"head_sha":"'+source+'","conclusion":"'+conclusion+'","name":"Build installer binaries"}').encode()
            with patch.dict(os.environ, {'BINARY_RUN_ID':'123', 'CONFIG_SHA256':'1'*64, 'LAUNCHER_SHA256':'2'*64}), patch.object(prepare.subprocess,'check_output',return_value=data):
                with self.assertRaisesRegex(ValueError, 'source/status'):
                    prepare.prepare(Path('.'), Path('.'), Path('unused'))

    @unittest.skipUnless(os.name == 'nt', 'Windows ConPTY fixture')
    def test_actual_console_cancels_then_closes_completion(self):
        import windows_console
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root/'console.ps1'
            script.write_text('[Console]::WriteLine("Reinstall existing Couch / Cancel"); 1..4 | ForEach-Object { $null = [Console]::ReadKey($true) }; [Console]::WriteLine("SESSION ENDED Enter / Esc close"); $null = [Console]::ReadKey($true); exit 0')
            with patch.dict(os.environ, {'LOCALAPPDATA':str(root/'owner')}):
                result = windows_console.run(script, root/'console.txt')
            self.assertTrue(result['cancel_selected'])
            self.assertTrue(result['completion_closed'])
            self.assertFalse(result['session_created'])

    @unittest.skipUnless(os.name == 'nt' and os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted', 'Ephemeral hosted Windows HTTPS fixture')
    def test_https_fixture_and_console_restore_runner_state(self):
        import subprocess
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root/'assets').mkdir(); (root/'launchers').mkdir()
            for name in ('couch-installer-host-windows-x64.exe', 'couch-installer-tui-windows-x64.exe', 'installer.json'):
                (root/'assets'/name).write_bytes(b'fixture')
            script = '''$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Net.Http
$client=[Net.Http.HttpClient]::new()
foreach ($name in @('couch-installer-host-windows-x64.exe','couch-installer-tui-windows-x64.exe','installer.json')) {
 $bytes=$client.GetByteArrayAsync("https://github.com/dangerouslaser/couch/releases/download/v0.1.0-alpha.20260910.24/$name").GetAwaiter().GetResult()
 if ([Text.Encoding]::UTF8.GetString($bytes) -ne 'fixture') { throw 'Fixture bytes differ' }
}
$client.Dispose()
[Console]::WriteLine('Reinstall existing Couch / Cancel')
1..4 | ForEach-Object { $null=[Console]::ReadKey($true) }
'''
            (root/'launchers/install.ps1').write_text(script)
            subprocess.run(['powershell','-NoProfile','-ExecutionPolicy','Bypass','-File',str(Path(__file__).with_name('windows_launcher.ps1')),'-Candidate',str(root)],check=True,timeout=160)
            self.assertTrue((root/'result.json').is_file())

    def test_symlinks_and_empty_artifacts_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'empty'; path.write_bytes(b'')
            with self.assertRaisesRegex(ValueError,'size'): prepare.digest(path)
            path.write_bytes(b'fixture')
            self.assertEqual(prepare.digest(path)['size'],7)


if __name__ == '__main__': unittest.main()
