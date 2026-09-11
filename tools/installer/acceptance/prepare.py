"""Admit frozen build artifacts and one independently pinned public descriptor."""
import base64
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys

SOURCE = os.environ.get('SOURCE_COMMIT', 'c09bb5a26cd22fbfa6425e8ec20872e37d673345')
VERSION = 'v0.1.0-alpha.20260910.24'
PLATFORMS = ('linux-x64', 'macos-universal', 'windows-x64')


def digest(path):
    if path.is_symlink() or not path.is_file(): raise ValueError('Expected regular artifact')
    data = path.read_bytes()
    if not 0 < len(data) <= 128*1024*1024: raise ValueError('Invalid artifact size')
    return {'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()}


def prepare(downloads, frozen, output):
    if not re.fullmatch('[0-9a-f]{40}', SOURCE): raise ValueError('Invalid independently pinned source')
    if not re.fullmatch('[0-9]{1,20}', os.environ['BINARY_RUN_ID']): raise ValueError('Invalid build run ID')
    for name in ('CONFIG_SHA256', 'LAUNCHER_SHA256'):
        if not re.fullmatch('[0-9a-f]{64}', os.environ[name]): raise ValueError('Invalid trusted hash')
    info = json.loads(subprocess.check_output(['gh', 'api', 'repos/dangerouslaser/couch/actions/runs/'+os.environ['BINARY_RUN_ID']]))
    if info['head_sha'] != SOURCE or info['conclusion'] != 'success' or info['name'] != 'Build installer binaries':
        raise ValueError('Build run source/status differs')
    if subprocess.check_output(['git', '-C', str(frozen), 'rev-parse', 'HEAD'], text=True).strip() != SOURCE:
        raise ValueError('Launcher source differs')
    config = base64.b64decode(os.environ['CONFIG_BASE64'], validate=True)
    if not 0 < len(config) <= 65536 or hashlib.sha256(config).hexdigest() != os.environ['CONFIG_SHA256']:
        raise ValueError('Public descriptor hash differs')
    metadata = json.loads(config)
    if metadata['source_commit'] != SOURCE or metadata['version'] != VERSION:
        raise ValueError('Public descriptor source/version differs')
    output.mkdir(mode=0o700)
    assets = output/'assets'; assets.mkdir()
    (assets/'installer.json').write_bytes(config)
    records = {}
    for platform in PLATFORMS:
        receipt_path = downloads/f'couch-installer-build-{platform}'/'build.json'
        if receipt_path.stat().st_size > 1024*1024: raise ValueError('Receipt too large')
        receipt = json.loads(receipt_path.read_bytes())
        expected_kind = 'couch-installer-universal-build' if platform == 'macos-universal' else 'couch-installer-native-build'
        if (receipt.get('schema'), receipt.get('kind'), receipt.get('source_commit'), receipt.get('platform')) != (1, expected_kind, SOURCE, platform):
            raise ValueError('Build receipt source/platform differs')
        if platform == 'macos-universal':
            if receipt['architectures'] != ['x86_64', 'arm64']: raise ValueError('Universal architectures differ')
            if set(receipt['inputs']) != {'macos-x64', 'macos-arm64'}: raise ValueError('Missing native universal inputs')
            for name, item in receipt['inputs'].items():
                native = downloads/f'couch-installer-build-{name}'/'build.json'
                if digest(native) != item['receipt'] or json.loads(native.read_bytes()) != item['build']:
                    raise ValueError('Universal input receipt differs')
                if item['build']['source_commit'] != SOURCE: raise ValueError('Mixed source commits')
        extension = '.exe' if platform == 'windows-x64' else ''
        for component in ('host', 'tui'):
            name = f'couch-installer-{component}'
            original = downloads/f'{name}-{platform}'/(name+extension)
            if digest(original) != receipt['binaries'][component]: raise ValueError('Binary hash differs')
            shutil.copyfile(original, assets/f'{name}-{platform}{extension}')
        records[platform] = receipt
    subprocess.run([sys.executable, str(frozen/'tools/release/installer_launchers.py'), '--assets', str(assets),
                    '--output', str(output/'launchers'), '--version', VERSION], check=True, timeout=30)
    launcher = output/'launchers/install.ps1'
    if digest(launcher)['sha256'] != os.environ['LAUNCHER_SHA256']:
        raise ValueError('Generated final launcher differs from independently pinned launcher')
    (output/'admission.json').write_text(json.dumps({'schema':1, 'source_commit':SOURCE,
        'binary_run_id':int(os.environ['BINARY_RUN_ID']), 'config':digest(assets/'installer.json'),
        'payload':metadata['payload'], 'launcher':digest(launcher), 'builds':records}, indent=2)+'\n')


if __name__ == '__main__':
    prepare(*(Path(p).resolve() for p in sys.argv[1:]))
