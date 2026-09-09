#!/usr/bin/env python3
"""Refresh the offline IRDB subset. Requires a local full-history upstream checkout."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

REVISION = 'd126fb1b6f1e114c52b4a8c19839ea65e3a9c24d'
BOUNDARY = '2319685f2cbf0cd3f809609622cade14d24fb819'
URL = 'https://github.com/Lucaslhm/Flipper-IRDB'


def generate(checkout):
    def git(*args):
        return subprocess.check_output(['git', '-C', str(checkout), *args])
    git('merge-base', '--is-ancestor', BOUNDARY, REVISION)
    old_blobs = {line.split()[2] for line in git('ls-tree', '-r', BOUNDARY).decode().splitlines()}
    changes = git('diff', '--name-status', '--find-renames=40%', BOUNDARY, REVISION, '--', '*.ir').decode().splitlines()
    entries = []
    for line in changes:
        status, path = line.split('\t', 1)
        parts = path.split('/')
        if status != 'A' or len(parts) < 3 or any(p.startswith('_') for p in parts):
            continue
        if parts[0] not in {'TVs', 'Audio_and_Video_Receivers', 'SoundBars', 'Streaming_Devices', 'Projectors', 'DVD', 'VCR'}:
            continue
        # Reject files whose earliest creation predates the explicit CC0 policy,
        # including delete/re-add paths; reject byte-identical older copies.
        additions = git('log', '--format=%H', '--diff-filter=A', REVISION, '--', path).decode().splitlines()
        if not additions:
            continue
        if subprocess.run(['git', '-C', str(checkout), 'merge-base', '--is-ancestor', BOUNDARY, additions[-1]], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
            continue
        blob = git('rev-parse', f'{REVISION}:{path}').decode().strip()
        if blob in old_blobs:
            continue
        content = git('show', f'{REVISION}:{path}')
        # Converted external databases need their own rights review.
        if any(marker in content.lower() for marker in (b'irdb.tk', b'lirc', b'irdb/irdb', b'converted from')):
            continue
        text = content.decode('utf-8-sig')
        if 'Filetype: IR signals file' not in text or 'Version: 1' not in text:
            continue
        entries.append(dict(id='irdb-' + hashlib.sha256(path.encode()).hexdigest()[:16], brand=parts[1].replace('_', ' '), device_type=parts[0].replace('_', ' '), model=Path(path).stem.replace('_', ' '), path=path, source_url=f'{URL}/blob/{REVISION}/{path}', introduced_commit=additions[-1], blob_sha1=blob, sha256=hashlib.sha256(content).hexdigest(), license='CC0-1.0', text=text))
    return dict(source=dict(name='Flipper-IRDB · post-CC0 subset', revision=REVISION, license='CC0-1.0', url=URL, license_boundary=BOUNDARY), codesets=entries)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('checkout', type=Path)
    parser.add_argument('--output', type=Path, default=Path('daemon/couch-confd/assets/ir/catalog.json'))
    args = parser.parse_args()
    payload = generate(args.checkout)
    args.output.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n')
    print(f"Prepared {len(payload['codesets'])} source-attributed codesets")
