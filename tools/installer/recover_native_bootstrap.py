#!/usr/bin/env python3
"""Offline admission and a boot-only recovery operation for failed native startup.

The CLI never opens USB. An explicitly selected, exclusively held operator
session may call recover() with its connected reader and audited writer factory.
"""
import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import stat

from couch_install import IDENTITY_PARTITIONS, MODEL, fingerprint, require, sync_directory
from enroll_android import official_inputs, check_layout

ORIGINALS = IDENTITY_PARTITIONS | {'boot', 'recovery', 'odmdtbo', 'logo'}
TRANSITIONS = {('created', 'inputs_verified'), ('inputs_verified', 'android_bound'),
               ('android_bound', 'originals_saved'), ('originals_saved', 'stage_boot_pending'),
               ('stage_boot_pending', 'failed'),
               # A stage that authenticated and then refused the plan wrote nothing;
               # its journal never reaches a writing phase, so boot is still the only
               # partition that differs from the saved originals.
               ('stage_boot_pending', 'stage_connected'), ('stage_connected', 'failed')}
# Reinstall sessions journal a Couch snapshot; fresh enrollments journal Android.
# Either way the saved originals are the device's own, verified by readback.
SNAPSHOT_FILES = {'Couch': 'current-couch-snapshot.json', 'Android': 'enrollment.json'}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def valid_hash(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None


def private_directory(path):
    path = Path(path).absolute()
    require(not path.is_symlink() and path.is_dir(), 'Expected private regular session directory')
    require(all(not (parent/'.git').exists() for parent in (path, *path.parents)),
            'Private recovery evidence must remain outside Git')
    info = path.stat()
    if os.name != 'nt':
        require(info.st_uid == os.geteuid() and info.st_mode & 0o077 == 0,
                'Recovery evidence directory must be owner-only')
    return path


def read(path, limit):
    flags = os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_BINARY', 0) | getattr(os, 'O_NONBLOCK', 0)
    before = path.lstat()
    require(stat.S_ISREG(before.st_mode), 'Expected regular recovery evidence')
    with os.fdopen(os.open(path, flags), 'rb') as stream:
        actual = os.fstat(stream.fileno())
        require((before.st_dev, before.st_ino, before.st_size) ==
                (actual.st_dev, actual.st_ino, actual.st_size), 'Recovery evidence replaced')
        require(actual.st_size <= limit, 'Oversized recovery evidence')
        data = stream.read(limit + 1)
        require(len(data) == actual.st_size, 'Recovery evidence size changed')
        return data


def file_hash(path, size):
    require(type(size) is int and 0 < size <= 64*1024*1024, 'Invalid original size')
    data = read(path, size)
    require(len(data) == size, 'Original backup size differs')
    return sha(data)


@dataclass(frozen=True)
class Proof:
    source: Path
    record: dict
    snapshot_sha256: str
    temporary_boot_sha256: str
    evidence_sha256: dict


def admit(source, expected_temporary_boot_sha256):
    source = private_directory(source)
    require(valid_hash(expected_temporary_boot_sha256), 'Explicit temporary boot hash required')
    snapshot = next((name for name in SNAPSHOT_FILES.values() if (source/name).is_file()), None)
    require(snapshot is not None and sum((source/name).is_file() for name in SNAPSHOT_FILES.values()) == 1,
            'Exactly one retained enrollment snapshot required')
    raw = read(source/snapshot, 65536)
    record = json.loads(raw)
    require(record.get('schema') == 1 and record.get('kind') == 'couch-device-enrollment'
            and record.get('model') == MODEL and SNAPSHOT_FILES.get(record.get('original_os')) == snapshot,
            'Snapshot file and recorded original OS disagree; never substitute an enrollment')
    require(isinstance(record.get('cid'), str) and re.fullmatch('[0-9a-f]{32}', record['cid'])
            and record['cid'] not in ('0'*32, 'f'*32), 'Invalid retained CID')
    require(type(record.get('capacity')) is int and record['capacity'] > 0, 'Invalid capacity')
    _, offsets = official_inputs(source/'owner-inputs')
    check_layout(record, offsets)
    require(set(record['originals']) == ORIGINALS and set(record['identity_sha256']) == IDENTITY_PARTITIONS,
            'Incomplete original inventory')
    for name, item in record['originals'].items():
        require(item['file'] == f'bootstrap-{name}.img' and valid_hash(item['sha256'])
                and item['size'] == record['partitions'][name]['size'], 'Invalid original record')
        require(file_hash(source/item['file'], item['size']) == item['sha256'], 'Original backup changed')
        if name in IDENTITY_PARTITIONS:
            require(record['identity_sha256'][name] == item['sha256'], 'Calibration record differs')
    require(record['originals']['boot']['sha256'] != expected_temporary_boot_sha256,
            'Temporary boot is not an original restoration image')
    names = sorted(p.name for p in source.glob('event-*.json'))
    require(1 <= len(names) <= 128 and names == [f'event-{i:05}.json' for i in range(len(names))],
            'Missing or noncontiguous native journal')
    originals = set(); saved = False; admitted = False; verified = False; rebound = False
    prior = 'created'; evidence_hashes = {snapshot: sha(raw)}
    for sequence, name in enumerate(names):
        data = read(source/name, 65536); event = json.loads(data); evidence_hashes[name] = sha(data)
        require(type(event.get('schema')) is int and event['schema'] == 1
                and type(event.get('sequence')) is int and event['sequence'] == sequence, 'Invalid journal sequence')
        phase, kind, evidence = event['phase'], event['kind'], event['evidence']
        if sequence == 0:
            require(phase == 'created' and kind == 'created', 'Missing session creation')
        elif kind == 'transition':
            require((prior, phase) in TRANSITIONS, 'Session progressed beyond recoverable bootstrap')
        else:
            require(kind == 'checkpoint' and phase == prior, 'Invalid journal checkpoint')
        label = evidence.get('event')
        if label == 'bootstrap_original_verified':
            target = evidence['target']; require(target in ORIGINALS and target not in originals
                and phase in ('inputs_verified', 'android_bound') and not saved, 'Invalid original checkpoint')
            original = record['originals'][target]
            require(all(evidence[k] == original[k] for k in ('sha256','size','file')), 'Original checkpoint mismatch')
            originals.add(target)
        if label == 'retained_enrollment_bound':
            require(phase == 'android_bound' and kind == 'transition' and evidence['original_os'] == 'Couch'
                    and record['original_os'] == 'Couch' and evidence['cid'] == record['cid'],
                    'Retained binding mismatch')
            rebound = True
        if label == 'android_bound':
            # Fresh enrollment: live Android was bound to this CID before any backup.
            require(phase == 'android_bound' and kind == 'transition' and record['original_os'] == 'Android'
                    and evidence['cid'] == record['cid'], 'Android binding mismatch')
            rebound = True
        if phase == 'originals_saved' and kind == 'transition':
            require(label == 'enrollment_complete' and evidence['enrollment_sha256'] == sha(raw)
                    and originals == ORIGINALS and rebound, 'Unverified original snapshot')
            saved = True
        if label == 'bootstrap_write_admitted':
            require(saved and phase == 'stage_boot_pending' and kind == 'transition'
                    and evidence['stage_sha256'] == expected_temporary_boot_sha256 and not admitted,
                    'Temporary boot admission mismatch')
            admitted = True
        if label == 'bootstrap_readback_verified':
            require(admitted and phase == 'stage_boot_pending' and kind == 'checkpoint'
                    and evidence['stage_sha256'] == expected_temporary_boot_sha256 and not verified,
                    'Temporary boot readback mismatch')
            verified = True
        if phase == 'failed':
            # installation_stopped: the host stopped before the stage ran a transaction.
            # transaction_stopped: the stage refused or abandoned the plan; the journal
            # above proves no writing phase was reached, so only boot differs.
            require(sequence == len(names)-1 and (label == 'installation_stopped'
                    or (label == 'transaction_stopped' and prior == 'stage_connected')),
                    'Failure must be final and precede any partition write')
        prior = phase
    require(saved and admitted and verified and prior == 'failed', 'No completed failed-bootstrap proof')
    return Proof(source, record, sha(raw), expected_temporary_boot_sha256, evidence_hashes)


def publish(directory, name, value):
    data = (json.dumps(value, sort_keys=True, indent=2)+'\n').encode()
    with (directory/name).open('xb') as stream:
        stream.write(data); stream.flush(); os.fsync(stream.fileno())
    sync_directory(directory)


def recover(proof, reader, writer_factory, output, *, current_boot_sha256=None, chain_receipt_sha256=None):
    """One boot write; caller owns exclusive USB, dependency pins and explicit restart.

    writer_factory(release, bundle, binding) must construct ConnectedMtkWriter on
    the SAME connected MTK instance as reader. No rediscovery, reconnect or retry.

    By default the live boot must still be the failed session's temporary image.
    After verified debug-stage transitions the boot slot holds the last debug
    image instead; the caller then passes that hash, attested by the pinned
    chain receipt it validated, as current_boot_sha256. The restoration source
    and every retained-partition gate are unchanged.
    """
    if current_boot_sha256 is None:
        current_boot_sha256 = proof.temporary_boot_sha256
    else:
        require(valid_hash(current_boot_sha256) and valid_hash(chain_receipt_sha256 or ''),
                'Explicit current debug boot and chain receipt hashes required')
    require(current_boot_sha256 != proof.record['originals']['boot']['sha256'],
            'Boot already holds the original image; nothing to restore')
    # Re-admit evidence immediately before consulting live hardware.
    checked = admit(proof.source, proof.temporary_boot_sha256)
    require(checked.record == proof.record and checked.evidence_sha256 == proof.evidence_sha256,
            'Recovery evidence changed after admission')
    observed = reader.description
    require(observed.get('hwcode') == 0x6580 and observed.get('cid_encoding') == 'mt6580-legacy-le32-registers'
            and observed['runtime_cid_sha256'] == sha(bytes.fromhex(proof.record['cid']))
            and observed['capacity'] == proof.record['capacity']
            and observed['partitions'] == proof.record['partitions'], 'Live hardware differs from failed session')
    require(reader.hash('boot') == current_boot_sha256, 'Current boot is not the verified temporary or debug image')
    retained = {name: item['sha256'] for name,item in proof.record['originals'].items() if name != 'boot'}
    for name, expected in sorted(retained.items()):
        require(reader.hash(name) == expected, 'Retained live partition changed')
    output = Path(output).absolute()
    private_directory(output.parent)
    output.mkdir(mode=0o700)  # Exclusive new recovery record; old session is untouched.
    publish(output, 'admitted.json', {'schema':1,'kind':'couch-native-bootstrap-restore',
            'snapshot_sha256':proof.snapshot_sha256,'temporary_boot_sha256':proof.temporary_boot_sha256,
            'current_boot_sha256':current_boot_sha256,'debug_chain_receipt_sha256':chain_receipt_sha256,
            'original_boot_sha256':proof.record['originals']['boot']['sha256'],
            'source_evidence_sha256':proof.evidence_sha256,'write_targets':['boot'],'complete':False})
    binding = {**{k:observed[k] for k in ('storage_id','runtime_cid_sha256','partitions')},
               'model':MODEL,'model_verified':True,'identity_sha256':fingerprint(proof.record['identity_sha256'])}
    original = proof.record['originals']['boot']
    release = {'schema':1,'model':MODEL,'partitions':proof.record['partitions'],'images':{'boot':original}}
    writer = writer_factory(release, proof.source, binding)
    try:
        # Construction may independently reread GPT; recheck all live gates on that facade.
        require(writer.description['runtime_cid_sha256'] == observed['runtime_cid_sha256']
                and writer.description['partitions'] == observed['partitions'], 'Writer session changed')
        require(writer.hash('boot') == current_boot_sha256, 'Boot changed before restore')
        for name, expected in sorted(retained.items()):require(writer.hash(name) == expected, 'Retained partition changed before restore')
        writer.write('boot', proof.source/original['file'])
        require(writer.hash('boot') == original['sha256'], 'Restored boot readback differs; do not retry')
        for name, expected in sorted(retained.items()):require(writer.hash(name) == expected, 'Retained partition changed after restore')
        publish(output,'verified.json',{'schema':1,'complete':True,'restored':['boot'],
                'boot_sha256':original['sha256'],'retained_sha256':retained,'restart_requested':False})
    finally:
        writer.close()
    return {'restored':['boot'],'verified':True,'restart_requested':False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--session', type=Path, required=True)
    parser.add_argument('--expected-temporary-boot-sha256', required=True)
    args = parser.parse_args()
    proof = admit(args.session, args.expected_temporary_boot_sha256)
    print(json.dumps({'admitted':True,'device_access':False,'originals_verified':len(proof.record['originals']),
                      'restoration_target':'boot','temporary_boot_sha256':proof.temporary_boot_sha256}))
