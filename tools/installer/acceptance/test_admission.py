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

    def test_symlinks_and_empty_artifacts_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'empty'; path.write_bytes(b'')
            with self.assertRaisesRegex(ValueError,'size'): prepare.digest(path)
            path.write_bytes(b'fixture')
            self.assertEqual(prepare.digest(path)['size'],7)


if __name__ == '__main__': unittest.main()
