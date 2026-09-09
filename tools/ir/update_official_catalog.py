#!/usr/bin/env python3
"""Package pinned MIT Flipper Devices IRDB as independently compressed records."""
import argparse
import gzip
import hashlib
import json
import math
import subprocess
from pathlib import Path
from urllib.parse import quote

REVISION = 'f7b15366521cc81ba11b341538f0097bddbb748b'
URL = 'https://github.com/flipperdevices/IRDB'
KINDS = {'TVs': 'TVs', 'DVD': 'DVD', 'A_V_receiver': 'Audio and Video Receivers', 'Box': 'Streaming Devices', 'Projector': 'Projectors', 'Fan':'Fans', 'Camera':'Cameras', 'Air_Purifiers':'Air Purifiers'}


def inspect(text):
    header = {}
    blocks = []
    for line in text.splitlines():
        line = line.lstrip('\ufeff').strip()
        if not line or line.startswith('#'):
            continue
        key, sep, value = line.partition(':')
        if not sep:
            raise ValueError('missing colon')
        if key == 'name':
            blocks.append({})
        dest = blocks[-1] if blocks else header
        if key.strip() in dest:
            raise ValueError('duplicate field')
        dest[key.strip()] = value.strip()
    if header.get('Filetype') != 'IR signals file' or header.get('Version') != '1' or not 1 <= len(blocks) <= 256:
        raise ValueError('unsupported header or command count')
    def little(s):
        values = s.split()
        if len(values) != 4:
            raise ValueError('not four bytes')
        return int.from_bytes(bytes(int(v,16) for v in values), 'little')
    def supported(block):
        try:
            if block.get('type') == 'raw':
                duty = float(block['duty_cycle'])
                values = [int(v) for v in block['data'].split()]
                return math.isfinite(duty) and abs(duty-0.33) <= 0.005 and 20000 <= int(block['frequency']) <= 60000 and 1 <= len(values) <= 1024 and all(v > 0 for v in values) and sum(values) <= 500000
            if block.get('type') != 'parsed':
                return False
            a,c = little(block['address']),little(block['command'])
            protocol = block['protocol']
            if protocol == 'NECext':
                return a < 65536 and c < 65536 and c >> 8 == (~c & 255)
            bits = {'NEC':(8,8),'RC5':(5,6),'RC6':(8,8),'SIRC':(5,7),'SIRC15':(8,7),'SIRC20':(13,7),'Samsung32':(8,8)}.get(protocol)
            return bool(bits and a < 2**bits[0] and c < 2**bits[1])
        except (ValueError,KeyError,OverflowError):
            return False
    return len(blocks),sum(supported(b) for b in blocks)


def generate(checkout, output):
    def git(*args):
        return subprocess.check_output(['git','-C',str(checkout),*args])
    if git('rev-parse','HEAD').decode().strip() != REVISION:
        raise ValueError(f'Checkout must be pinned at {REVISION}')
    records = []
    packed = bytearray()
    excluded = []
    prefix = Path('database/categories')
    for line in git('ls-tree','-r',REVISION,'--',str(prefix)).decode().splitlines():
        attrs,path = line.split('\t',1)
        if not path.endswith('.ir'):
            continue
        # Read committed bytes, never an untracked or edited working-tree file.
        content = git('show',f'{REVISION}:{path}')
        try:
            text = content.decode('utf-8-sig')
            if len(content) > 256*1024:
                raise ValueError('source exceeds import bound')
            count,supported = inspect(text)
        except (ValueError,UnicodeError) as error:
            excluded.append({'path':path,'reason':str(error)})
            continue
        parts = Path(path).relative_to(prefix).parts
        zipped = gzip.compress(content,compresslevel=9,mtime=0)
        records.append(dict(id='flipper-'+hashlib.sha256(path.encode()).hexdigest()[:16],brand=parts[1].replace('_',' '),device_type=KINDS.get(parts[0],parts[0]),model=Path(path).stem.replace('_',' '),path=path,source_url=f'{URL}/blob/{REVISION}/{quote(path)}',license='MIT',blob_sha1=attrs.split()[2],sha256=hashlib.sha256(content).hexdigest(),commands_count=count,supported_commands=supported,supported=supported>0,offset=len(packed),length=len(zipped)))
        packed.extend(zipped)
    source = dict(name='Flipper Devices IRDB',revision=REVISION,license='MIT',url=URL)
    output.mkdir(parents=True,exist_ok=True)
    (output/'official-index.json').write_text(json.dumps(dict(source=source,codesets=records,excluded=excluded),ensure_ascii=False,separators=(',',':'))+'\n')
    (output/'official-data.irpack').write_bytes(packed)
    (output/'LICENSE-Flipper-MIT.txt').write_bytes(git('show',f'{REVISION}:LICENSE'))
    print(f'{len(records)} models, {sum(r["supported_commands"] for r in records)} encodable commands; {len(excluded)} rejected files; {len(packed)} packed bytes')

if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('checkout',type=Path)
    parser.add_argument('--output',type=Path,default=Path('daemon/couch-confd/assets/ir'))
    args=parser.parse_args()
    generate(args.checkout,args.output)
