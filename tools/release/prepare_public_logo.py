#!/usr/bin/env python3
"""Export Couch's own canonical boot frame without any owner's logo partition."""
import argparse
import hashlib
import json
from pathlib import Path
from PIL import Image

from clean_stage import require


def prepare(output):
    require(not output.exists(), 'Public logo output must be new')
    source = Path(__file__).resolve().parents[2] / 'assets/boot/couch.png'
    with Image.open(source) as image:
        require(image.size == (480, 800), 'Expected native HA100 boot frame')
        data = image.convert('RGBA').tobytes('raw', 'BGRA')
    output.mkdir(parents=True, mode=0o700)
    (output / 'logo.bgra').write_bytes(data)
    result = {'schema': 1, 'kind': 'couch-public-logo-frame', 'file': 'logo.bgra',
              'size': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
              'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
              'assembly': 'Replace frame zero only in the verified owner-local original logo partition.'}
    (output / 'logo.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    prepare(parser.parse_args().output)
