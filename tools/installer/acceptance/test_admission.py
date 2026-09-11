import base64
import hashlib
import json
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

    def test_independent_payload_and_host_pins_with_verified_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root/'downloads'
            records = {}
            for platform in ('linux-x64', 'macos-x64', 'macos-arm64', 'macos-universal', 'windows-x64'):
                record = {'schema': 1, 'kind': 'couch-installer-native-build',
                          'source_commit': prepare.SOURCE, 'platform': platform, 'binaries': {}}
                for component in ('host', 'tui'):
                    name = f'couch-installer-{component}'
                    artifact = downloads/f'{name}-{platform}'/(name+('.exe' if platform == 'windows-x64' else ''))
                    artifact.parent.mkdir(parents=True)
                    artifact.write_bytes(f'{platform}-{component}'.encode())
                    record['binaries'][component] = prepare.digest(artifact)
                if platform == 'macos-universal':
                    record.update(kind='couch-installer-universal-build', architectures=['x86_64', 'arm64'],
                                  inputs={name: {'receipt': prepare.digest(downloads/f'couch-installer-build-{name}'/'build.json'),
                                                 'build': records[name]} for name in ('macos-x64', 'macos-arm64')})
                receipt = downloads/f'couch-installer-build-{platform}'/'build.json'
                receipt.parent.mkdir()
                receipt.write_text(json.dumps(record))
                records[platform] = record
            launcher = b'fixture launcher'
            metadata = {'source_commit': prepare.PAYLOAD_SOURCE, 'version': prepare.VERSION,
                        'payload': {'url': 'https://example.invalid/payload', 'size': 1, 'sha256': 'a'*64}}
            def invoke(output, config_source=prepare.PAYLOAD_SOURCE, run_source=prepare.SOURCE,
                       generator_source=prepare.SOURCE, launcher_hash=None):
                config = json.dumps(dict(metadata, source_commit=config_source)).encode()
                env = {'BINARY_RUN_ID': '123', 'CONFIG_BASE64': base64.b64encode(config).decode(),
                       'CONFIG_SHA256': hashlib.sha256(config).hexdigest(),
                       'LAUNCHER_SHA256': launcher_hash or hashlib.sha256(launcher).hexdigest()}
                def generate(*args, **kwargs):
                    (output/'launchers').mkdir()
                    (output/'launchers/install.ps1').write_bytes(launcher)
                run = json.dumps({'head_sha': run_source, 'conclusion': 'success', 'name': 'Build installer binaries'})
                with patch.dict(os.environ, env), patch.object(prepare.subprocess, 'check_output', side_effect=[run, generator_source+'\n']), patch.object(prepare.subprocess, 'run', side_effect=generate):
                    prepare.prepare(downloads, root/'frozen', output)
            self.assertNotEqual(prepare.SOURCE, prepare.PAYLOAD_SOURCE)
            invoke(root/'accepted')
            admission = json.loads((root/'accepted/admission.json').read_text())
            self.assertEqual(admission['source_commit'], prepare.SOURCE)
            self.assertEqual(admission['payload_source_commit'], prepare.PAYLOAD_SOURCE)
            for name, kwargs, message in (
                ('payload', {'config_source': prepare.SOURCE}, 'descriptor source'),
                ('host', {'run_source': prepare.PAYLOAD_SOURCE}, 'source/status'),
                ('generator', {'generator_source': prepare.PAYLOAD_SOURCE}, 'Launcher source'),
                ('launcher', {'launcher_hash': '0'*64}, 'pinned launcher'),
            ):
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, message):
                    invoke(root/name, **kwargs)
            receipt = downloads/'couch-installer-build-linux-x64/build.json'
            record = json.loads(receipt.read_text())
            record['source_commit'] = prepare.PAYLOAD_SOURCE
            receipt.write_text(json.dumps(record))
            with self.assertRaisesRegex(ValueError, 'receipt source'):
                invoke(root/'receipt')

    def test_invalid_payload_pin_rejected_before_external_access(self):
        with patch.object(prepare, 'PAYLOAD_SOURCE', '../untrusted'), patch.object(prepare.subprocess, 'check_output') as external:
            with self.assertRaisesRegex(ValueError, 'payload source'):
                prepare.prepare(Path('.'), Path('.'), Path('unused'))
            external.assert_not_called()

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
