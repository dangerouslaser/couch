#!/usr/bin/env python3
"""Prepare a noninstallable raw userdata ext4 file; never opens a device."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import tempfile
import uuid

from clean_stage import checksum, require
from package_closure import verify
from prepare_rootfs import normalize

MKE2FS_CONFIG = b'''[defaults]
    blocksize = 4096
    inode_size = 256
    inode_ratio = 16384
    enable_periodic_fsck = 0
[fs_types]
    ext4 = {
    }
    default = {
    }
'''


def validate_geometry(value):
    require(value.get('schema') == 1 and value.get('partition') == 'userdata',
            'Only explicit userdata geometry is accepted')
    require(value.get('source') in ('observed', 'fixture'), 'Label geometry observed or fixture')
    require(isinstance(value.get('evidence_sha256'), str) and
            re.fullmatch('[0-9a-f]{64}', value['evidence_sha256']), 'Missing geometry evidence hash')
    for name in ('offset', 'size'):
        require(type(value.get(name)) is int and value[name] > 0 and value[name] % 4096 == 0,
                'Geometry must use positive 4096-byte-aligned byte values')
    require(32 * 1024**2 <= value['size'] <= 64 * 1024**3, 'Unsupported userdata image size')
    require(value['offset'] + value['size'] < 2**63, 'Geometry overflow')
    # The assembler accepts only geometry/hash metadata, never private IDs.
    require(set(value) == {'schema', 'partition', 'source', 'evidence_sha256', 'offset', 'size'},
            'Geometry must not include private device identifiers or extra fields')
    return value


def file_hash(path):
    result = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(4 * 1024**2), b''):
            result.update(block)
    return result.hexdigest()


def validate_image(path, size):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size == size,
            'Image must be a full-size regular raw file')
    with path.open('rb') as source:
        source.seek(1024)
        block = source.read(1024)
    require(len(block) == 1024 and block[56:58] == b'\x53\xef', 'Missing ext4 superblock')
    blocks = struct.unpack_from('<I', block, 4)[0]
    log_block = struct.unpack_from('<I', block, 24)[0]
    require(log_block == 2 and blocks * 4096 == size, 'Filesystem geometry differs from partition')
    compat = struct.unpack_from('<I', block, 92)[0]
    require(compat == 0x2c, 'Unexpected compatible filesystem features')
    incompat = struct.unpack_from('<I', block, 96)[0]
    ro_compat = struct.unpack_from('<I', block, 100)[0]
    require(incompat == 0x42 and ro_compat == 0x73,
            'Unexpected filesystem features for the legacy kernel')


def prepare(staging_dir, tools_dir, geometry, output):
    geometry = validate_geometry(geometry)
    require(not output.exists(), 'Output directory must be new')
    staging = json.loads((staging_dir / 'staging.json').read_text())
    require(staging.get('kind') == 'couch-packaged-staging' and staging.get('installable') is False,
            'Expected reviewed packaged staging input')
    source = staging_dir / 'rootfs-staging.tar.gz'
    require(source.is_file() and not source.is_symlink(), 'Expected regular rootfs archive')
    data = source.read_bytes()
    require(checksum(data) == staging['archive_sha256'], 'Rootfs archive checksum mismatch')
    epoch = staging['source_date_epoch']
    require(type(epoch) is int and 0 <= epoch <= 0xffffffff, 'Invalid build timestamp')
    normalize(data, epoch)  # Rescan archive safety/defaults without host extraction.
    tools_dir = tools_dir.resolve()
    tool_manifest = verify(tools_dir)
    require(tool_manifest['architecture'] == 'x86_64', 'Image tools must be x86_64')
    image = tool_manifest['builder_image']
    require(re.fullmatch(r'alpine@sha256:[0-9a-f]{64}', image), 'Unpinned builder')
    filesystem_uuid = str(uuid.uuid5(uuid.NAMESPACE_URL,
        'couch-userdata:' + staging['archive_sha256'] + ':' + str(geometry['size'])))
    output.mkdir()
    helper = Path(__file__).with_name('build_ext4.sh').resolve()
    with tempfile.TemporaryDirectory(prefix='couch-ext4-input-') as temp:
        inputs = Path(temp)
        (inputs / 'rootfs-staging.tar.gz').write_bytes(data)
        (inputs / 'mke2fs.conf').write_bytes(MKE2FS_CONFIG)
        (inputs / 'tool-hashes.txt').write_text(''.join(
            f"{p['sha256']}  packages/{p['filename']}\n" for p in tool_manifest['packages']))
        subprocess.run(['docker', 'run', '--rm', '--platform=linux/amd64', '--network=none',
                        '--read-only', '--cap-drop=ALL', '--cap-add=CHOWN',
                        '--cap-add=FOWNER', '--cap-add=DAC_OVERRIDE', '--security-opt=no-new-privileges',
                        '--tmpfs', '/tmp:rw,exec,nosuid,nodev,size=512m',
                        '--mount', f'type=bind,src={inputs},dst=/input,readonly',
                        '--mount', f'type=bind,src={tools_dir},dst=/tools,readonly',
                        '--mount', f'type=bind,src={helper},dst=/build.sh,readonly',
                        '--mount', f'type=bind,src={output.resolve()},dst=/out',
                        image, 'sh', '/build.sh', str(epoch), str(geometry['size'] // 4096),
                        filesystem_uuid], check=True)
    artifact = output / 'userdata.ext4'
    validate_image(artifact, geometry['size'])
    result = {'schema': 1, 'kind': 'couch-userdata-image-preparation', 'installable': False,
              'geometry': geometry, 'source_date_epoch': epoch, 'filesystem_uuid': filesystem_uuid,
              'rootfs_archive_sha256': staging['archive_sha256'],
              'staging_manifest_sha256': file_hash(staging_dir / 'staging.json'),
              'tool_closure_sha256': file_hash(tools_dir / 'closure.json'), 'builder_image': image,
              'image': {'path': 'userdata.ext4', 'format': 'raw-ext4',
                        'size': geometry['size'], 'sha256': file_hash(artifact)},
              'validation': ['e2fsck -fn', 'superblock size and legacy feature validation'],
              'pending': ['complete reviewed runtime/vendor artifacts', 'signed release inventory',
                          'physical boot and recovery validation']}
    (output / 'image.json').write_text(json.dumps(result, sort_keys=True, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('staging', type=Path)
    parser.add_argument('tools', type=Path)
    parser.add_argument('geometry', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = prepare(args.staging, args.tools, json.loads(args.geometry.read_text()), args.output)
    print(f"Prepared noninstallable raw ext4 file: {result['image']['size']} bytes")


if __name__ == '__main__':
    main()
