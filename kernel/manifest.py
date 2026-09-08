#!/usr/bin/env python3
"""Archive effective inputs and output hashes beside the kernel."""
import hashlib
import json
import subprocess
import sys
from pathlib import Path


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def sha(path):
    with Path(path).open('rb') as source:
        digest = hashlib.sha256()
        for block in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def main():
    tree, output, profile, container = sys.argv[1:]
    out = Path(output)
    manifest = {
        'profile': profile,
        'source_commit': command('git', '-C', tree, 'rev-parse', 'HEAD'),
        'source_status': command('git', '-C', tree, 'status', '--porcelain'),
        'source_diff_sha256': hashlib.sha256(subprocess.check_output(
            ['git', '-C', tree, 'diff', 'HEAD'])).hexdigest(),
        'container': command('docker', 'image', 'inspect', container, '--format', '{{.Id}}'),
        'compiler': (out / 'compiler.txt').read_text(),
        'compiler_sha256': (out / 'compiler.sha256').read_text().split()[0],
        'sha256': {name: sha(out / name) for name in
                   ['.config', 'arch/arm/boot/zImage', 'vmlinux', 'System.map']},
    }
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(manifest, indent=2))


if __name__ == '__main__':
    main()
