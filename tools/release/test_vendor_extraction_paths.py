"""Exercise the real extraction shell with fake Docker/debugfs, never a device."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class VendorExtractionPathsTests(unittest.TestCase):
    def test_docker_path_preserves_both_context_directories_and_system_etc(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'tools').mkdir(); (root / 'bin').mkdir(); (root / 'images').mkdir()
            shutil.copy(Path(__file__).resolve().parents[1] / 'extract-vendor.sh', root / 'tools/extract-vendor.sh')
            (root / 'tools/env.sh').write_text('BACKUP_DIR="$TEST_ROOT/images"\n')
            for name in ('vendor.img', 'system.img'):
                (root / 'images' / name).write_bytes(b'fixture')
            docker = root / 'bin/docker'
            docker.write_text('''#!/usr/bin/env python3
import os, subprocess, sys
if sys.argv[1] == 'info': sys.exit(0)
assert sys.argv[1] == 'run'
script=sys.argv[-1].replace('/w/',os.environ['TEST_ROOT']+'/').replace('/img/',os.environ['TEST_ROOT']+'/images/')
sys.exit(subprocess.call(['sh','-c',script]))
''')
            apk = root / 'bin/apk'; apk.write_text('#!/bin/sh\nexit 0\n')
            debugfs = root / 'bin/debugfs'
            debugfs.write_text('''#!/usr/bin/env python3
from pathlib import Path
import shlex, sys
op, source, destination=shlex.split(sys.argv[2])
path=Path(destination)
if op == 'dump':
 path.write_bytes(b'fixture')
else:
 assert op == 'rdump'
 path=path/Path(source).name
 path.mkdir(exist_ok=True)
 name=('nonplat_property_contexts' if 'vendor.img' in sys.argv[-1] else 'plat_property_contexts') if source == '/etc/selinux' else 'fixture.bin'
 (path/name).write_bytes(b'fixture')
''')
            for path in (docker, apk, debugfs):
                path.chmod(0o755)
            result = subprocess.run(['sh', str(root / 'tools/extract-vendor.sh')],
                env={**os.environ, 'TEST_ROOT': str(root), 'PATH': str(root/'bin')+os.pathsep+os.environ['PATH']},
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            out = root / 'build/vendor-bundle'
            for name in ('vendor/etc/selinux/nonplat_property_contexts',
                         'system/etc/selinux/plat_property_contexts', 'system/etc/ld.config.txt'):
                self.assertEqual((out / name).read_bytes(), b'fixture')
            self.assertFalse((out / 'vendor/selinux').exists())
            self.assertFalse((out / 'system/selinux').exists())


if __name__ == '__main__':
    unittest.main()
