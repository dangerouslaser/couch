"""Admit one boot-only transition from a retained failed installer session.

No capture/bootstrap runner is called. Live transport and its exclusive lease
belong to the native supervisor. This module never discovers USB or reboots.
"""
from dataclasses import dataclass
import copy
import json
from pathlib import Path

from couch_install import MODEL, fingerprint, require
import recover_native_bootstrap as recovery

MAX_COMPLETED_CHAIN_DEPTH = 4
LEGACY_INPUT_KEYS = {
    'source', 'temporary_boot_sha256', 'original_boot_sha256', 'snapshot_sha256',
    'image', 'image_sha256', 'metadata', 'metadata_sha256',
}


@dataclass(frozen=True)
class TransitionProof:
    retained: recovery.Proof
    original_boot_sha256: str
    snapshot_sha256: str
    image: Path
    image_sha256: str
    metadata: Path
    metadata_sha256: str


@dataclass(frozen=True)
class ChainedTransitionProof:
    """A distinct proof for updating an already verified debug boot.

    The parent receipt remains the only authority for the historical failed
    installer session.  In particular, this proof never changes the failed
    session's temporary-image pin.
    """
    parent: TransitionProof
    parent_receipt_sha256: str
    previous_boot_sha256: str
    image: Path
    image_sha256: str
    metadata: Path
    metadata_sha256: str
    candidate_receipt: Path
    candidate_receipt_sha256: str
    source_commit: str
    stage_base_commit: str
    probe_sha256: str
    config: dict


@dataclass(frozen=True)
class CompletedTransitionProof:
    """A verified receipt link ending at one concrete current debug boot."""
    parent: TransitionProof
    receipt_sha256: str
    boot_sha256: str
    chained: bool


def admit_target(*, image, image_sha256, metadata, metadata_sha256):
    """Validate a private, non-installable debug image without session state."""
    for pin in (image_sha256, metadata_sha256):
        require(recovery.valid_hash(pin), 'Explicit transition evidence hash required')
    image, metadata = Path(image).absolute(), Path(metadata).absolute()
    raw = recovery.read(metadata, 1024*1024)
    require(recovery.sha(raw) == metadata_sha256, 'Debug metadata differs from reviewed pin')
    manifest = json.loads(raw)
    require(manifest.get('schema') == 1 and manifest.get('kind') == 'private-ram-wifi-debug-stage'
            and manifest.get('private_only') is True and manifest.get('installable') is False
            and manifest.get('storage_operations') == []
            and manifest.get('sha256') == image_sha256
            and manifest.get('size') == 16*1024*1024,
            'Expected independently pinned debug-only stage metadata')
    data = recovery.read(image, 16*1024*1024)
    require(len(data) == manifest['size'] and data.startswith(b'ANDROID!')
            and recovery.sha(data) == image_sha256,
            'Debug image header/hash differs')
    return image, metadata, manifest


def admit(source, *, temporary_boot_sha256, original_boot_sha256,
          snapshot_sha256, image, image_sha256, metadata, metadata_sha256):
    for pin in (original_boot_sha256, snapshot_sha256):
        require(recovery.valid_hash(pin), 'Explicit transition evidence hash required')
    retained = recovery.admit(source, temporary_boot_sha256)
    require(retained.snapshot_sha256 == snapshot_sha256, 'Retained original snapshot differs from pin')
    require(retained.record['originals']['boot']['sha256'] == original_boot_sha256,
            'Original boot differs from retained baseline pin')
    require(image_sha256 not in (original_boot_sha256, temporary_boot_sha256),
            'Debug transition must use a distinct debug image')
    image, metadata, manifest = admit_target(image=image, image_sha256=image_sha256,
                                              metadata=metadata, metadata_sha256=metadata_sha256)
    require(manifest['size'] == retained.record['partitions']['boot']['size'],
            'Debug image size differs from retained boot partition')
    return TransitionProof(retained, original_boot_sha256, snapshot_sha256,
                           image, image_sha256, metadata, metadata_sha256)


def topology(bus, ports):
    require(type(bus) is int and 0 < bus <= 255 and isinstance(ports, list)
            and 1 <= len(ports) <= 7 and all(type(p) is int and 0 < p <= 255 for p in ports),
            'Invalid pinned physical topology')
    return {'bus': bus, 'ports': ports}


def receipt_evidence(proof, bus, ports):
    record = proof.retained.record
    return {'source_session': str(proof.retained.source),
        'source_evidence_sha256': proof.retained.evidence_sha256,
        'snapshot_sha256': proof.snapshot_sha256, 'original_boot_sha256': proof.original_boot_sha256,
        'previous_boot_sha256': proof.retained.temporary_boot_sha256,
        'debug_boot_sha256': proof.image_sha256, 'debug_metadata_sha256': proof.metadata_sha256,
        'cid': record['cid'], 'runtime_cid_sha256': recovery.sha(bytes.fromhex(record['cid'])),
        'cid_encoding': 'mt6580-legacy-le32-registers', 'hwcode': 0x6580,
        'capacity': record['capacity'], 'partitions': record['partitions'],
        'topology': topology(bus, ports)}


def _validated_legacy_receipt(config, bus, ports):
    """Return the immutable parent proof after checking every receipt member."""
    require(isinstance(config, dict) and set(config) == {'receipt', 'sha256', 'inputs'},
            'Pinned completed transition receipt required')
    require(recovery.valid_hash(config['sha256']), 'Explicit completed receipt SHA-256 required')
    require(isinstance(config['inputs'], dict) and set(config['inputs']) == LEGACY_INPUT_KEYS,
            'Legacy completed receipt inputs differ from the retained admission shape')
    path = Path(config['receipt'])
    require(path.is_absolute(), 'Receipt path must be absolute')
    recovery.private_directory(path.parent)
    raw = recovery.read(path, 256*1024)
    require(recovery.sha(raw) == config['sha256'], 'Completed receipt differs from pin')
    value = json.loads(raw)
    proof = admit(**config['inputs'])
    require(value.get('schema') == 2 and value.get('kind') == 'couch-wifi-debug-transition-completed'
            and value.get('complete') is True and value.get('restart_acknowledged') is True,
            'Incomplete debug transition')
    verified_raw = recovery.read(path.parent/'verified.json', 256*1024)
    require(recovery.sha(verified_raw) == value.get('verified_receipt_sha256'), 'Readback receipt changed')
    verified = json.loads(verified_raw)
    retained = {n: item['sha256'] for n, item in proof.retained.record['originals'].items() if n != 'boot'}
    expected = {'schema': 2, 'kind': 'couch-wifi-debug-transition', 'complete': True,
        **receipt_evidence(proof, bus, ports), 'written': ['boot'],
        'boot_sha256': proof.image_sha256, 'boot_readback_sha256': proof.image_sha256,
        'retained_before_sha256': retained, 'retained_sha256': retained, 'restart_requested': False}
    require(verified == expected and value.get('verification') == verified,
            'Completed transition identity, topology, pins or retained checks differ')
    for name, expected_ack in [('restart-requested.json', False), ('restart-acknowledged.json', True)]:
        raw_ack = recovery.read(path.parent/name, 4096)
        require(recovery.sha(raw_ack) == value.get(name + '_sha256')
                and json.loads(raw_ack) == {'requested': True, 'acknowledged': expected_ack},
                'Restart evidence missing or changed')
    return proof, value, verified


def _completed_identity(config, chain, depth, seen):
    """Open exactly one pinned receipt and reject recursive/path cycles early."""
    expected = {'chain', 'receipt', 'sha256', 'inputs'} if chain else {'receipt', 'sha256', 'inputs'}
    require(isinstance(config, dict), 'Pinned completed transition receipt required')
    require(set(config) == expected and (config.get('chain') is True if chain else True),
            'Pinned completed transition receipt required')
    require(recovery.valid_hash(config['sha256']), 'Explicit completed receipt SHA-256 required')
    require(depth < MAX_COMPLETED_CHAIN_DEPTH, 'Completed debug receipt chain exceeds bounded depth')
    path = Path(config['receipt'])
    require(path.is_absolute(), 'Receipt path must be absolute')
    recovery.private_directory(path.parent)
    raw = recovery.read(path, 256*1024)
    require(recovery.sha(raw) == config['sha256'], 'Completed receipt differs from pin')
    identity = (str(path.resolve()), config['sha256'])
    require(identity not in seen, 'Completed debug receipt chain contains a cycle')
    return path, raw, seen | {identity}


def _validated_completed_receipt(config, bus, ports, *, depth=0, seen=frozenset()):
    """Validate a schema-2 root or bounded schema-3 chain without flattening it."""
    if isinstance(config, dict) and set(config) == {'receipt', 'sha256', 'inputs'}:
        _completed_identity(config, False, depth, seen)
        proof, _, verified = _validated_legacy_receipt(config, bus, ports)
        return CompletedTransitionProof(proof, config['sha256'], verified['debug_boot_sha256'], False)

    path, raw, next_seen = _completed_identity(config, True, depth, seen)
    value = json.loads(raw)
    require(isinstance(config['inputs'], dict), 'Chained completed receipt inputs must be an object')
    proof = admit_chained(config['inputs'], bus, ports, _depth=depth + 1, _seen=next_seen)
    require(value.get('schema') == 3 and value.get('kind') == 'couch-wifi-debug-chained-transition-completed'
            and value.get('complete') is True and value.get('restart_acknowledged') is True,
            'Incomplete chained debug transition')
    verified_raw = recovery.read(path.parent/'verified.json', 256*1024)
    require(recovery.sha(verified_raw) == value.get('verified_receipt_sha256'), 'Chained readback receipt changed')
    verified = json.loads(verified_raw)
    retained = {n: item['sha256'] for n, item in proof.parent.retained.record['originals'].items() if n != 'boot'}
    expected = {'schema': 3, 'kind': 'couch-wifi-debug-chained-transition', 'complete': True,
        **chained_receipt_evidence(proof, bus, ports), 'written': ['boot'],
        'boot_sha256': proof.image_sha256, 'boot_readback_sha256': proof.image_sha256,
        'retained_before_sha256': retained, 'retained_sha256': retained, 'restart_requested': False}
    require(verified == expected and value.get('verification') == verified,
            'Chained transition identity, parent, pins or retained checks differ')
    for name, expected_ack in [('restart-requested.json', False), ('restart-acknowledged.json', True)]:
        raw_ack = recovery.read(path.parent/name, 4096)
        require(recovery.sha(raw_ack) == value.get(name + '_sha256')
                and json.loads(raw_ack) == {'requested': True, 'acknowledged': expected_ack},
                'Chained restart evidence missing or changed')
    return CompletedTransitionProof(proof.parent, config['sha256'], proof.image_sha256, True)


def validate_receipt(config, bus, ports):
    """Offline gate; pinned historical proof, not fresh stage attestation."""
    _validated_legacy_receipt(config, bus, ports)
    return {'validated': True, 'device_access': False, 'receipt_sha256': config['sha256']}


def _candidate_receipt(config):
    require(isinstance(config, dict) and set(config) == {
        'parent', 'candidate_receipt', 'candidate_receipt_sha256', 'source_commit',
        'stage_base_commit', 'probe_sha256', 'image', 'image_sha256', 'metadata', 'metadata_sha256'},
        'Pinned parent receipt and complete candidate binding required')
    for name in ('candidate_receipt_sha256', 'probe_sha256', 'image_sha256', 'metadata_sha256'):
        require(recovery.valid_hash(config[name]), 'Explicit chained transition hash required')
    for name in ('source_commit', 'stage_base_commit'):
        require(isinstance(config[name], str) and len(config[name]) == 40
                and all(c in '0123456789abcdef' for c in config[name]),
                'Explicit chained source commit required')
    path = Path(config['candidate_receipt'])
    require(path.is_absolute(), 'Candidate receipt path must be absolute')
    recovery.private_directory(path.parent)
    raw = recovery.read(path, 256*1024)
    require(recovery.sha(raw) == config['candidate_receipt_sha256'], 'Candidate receipt differs from pin')
    value = json.loads(raw)
    require(value.get('schema') == 1 and value.get('kind') == 'couch-private-wifi-debug-lifecycle-candidate'
            and value.get('installable') is False and value.get('device_access') is False
            and value.get('physical_boot_verified') is False,
            'Expected private offline debug candidate receipt')
    binding = value.get('source_binding')
    require(isinstance(binding, dict) and binding.get('partial_overlay') is True
            and binding.get('local_source_commit') == config['source_commit']
            and binding.get('stage_base_commit') == config['stage_base_commit'],
            'Candidate source binding differs from pinned chain')
    output = value.get('outputs', {})
    image_name = Path(output.get('image', {}).get('path', ''))
    metadata_name = Path(output.get('metadata', {}).get('path', ''))
    require(not image_name.is_absolute() and '..' not in image_name.parts
            and not metadata_name.is_absolute() and '..' not in metadata_name.parts
            and path.parent / image_name == Path(config['image'])
            and output.get('image', {}).get('sha256') == config['image_sha256']
            and path.parent / metadata_name == Path(config['metadata'])
            and output.get('metadata', {}).get('sha256') == config['metadata_sha256'],
            'Candidate output binding differs from pinned chain')
    probe = value.get('approved_overlay', {}).get('files', {}).get('stage/probe', {})
    require(probe.get('sha256') == config['probe_sha256'], 'Candidate debug probe pin differs')
    return path, value


def admit_chained(config, bus, ports, *, _depth=0, _seen=frozenset()):
    """Admit a bounded debug-to-debug update without redefining failed-session proof."""
    candidate_path, _ = _candidate_receipt(config)
    completed = _validated_completed_receipt(config['parent'], bus, ports,
                                              depth=_depth, seen=_seen)
    parent, previous = completed.parent, completed.boot_sha256
    require(previous != parent.retained.temporary_boot_sha256
            and previous != parent.original_boot_sha256,
            'Parent receipt does not establish a distinct current debug boot')
    image, metadata, manifest = admit_target(image=config['image'], image_sha256=config['image_sha256'],
                                              metadata=config['metadata'], metadata_sha256=config['metadata_sha256'])
    require(manifest['size'] == parent.retained.record['partitions']['boot']['size']
            and config['image_sha256'] not in (previous, parent.original_boot_sha256,
                                                parent.retained.temporary_boot_sha256),
            'Chained debug image is not a distinct boot-sized target')
    return ChainedTransitionProof(parent, completed.receipt_sha256, previous, image,
                                  config['image_sha256'], metadata, config['metadata_sha256'],
                                  candidate_path, config['candidate_receipt_sha256'],
                                  config['source_commit'], config['stage_base_commit'],
                                  config['probe_sha256'], copy.deepcopy(config))


def chained_receipt_evidence(proof, bus, ports):
    evidence = receipt_evidence(proof.parent, bus, ports)
    evidence.update({'parent_completed_receipt_sha256': proof.parent_receipt_sha256,
                     'parent_debug_boot_sha256': proof.previous_boot_sha256,
                     'previous_boot_sha256': proof.previous_boot_sha256,
                     'debug_boot_sha256': proof.image_sha256,
                     'debug_metadata_sha256': proof.metadata_sha256,
                     'candidate_receipt_sha256': proof.candidate_receipt_sha256,
                     'candidate_source_commit': proof.source_commit,
                     'candidate_stage_base_commit': proof.stage_base_commit,
                     'candidate_probe_sha256': proof.probe_sha256})
    return evidence


def validate_chained_receipt(config, bus, ports):
    completed = _validated_completed_receipt(config, bus, ports)
    require(completed.chained, 'Pinned chained completed receipt required')
    return {'validated': True, 'device_access': False, 'receipt_sha256': completed.receipt_sha256}


def complete_receipt(output):
    """Only called after independently verified write and acknowledged boot."""
    output = Path(output)
    raw = recovery.read(output/'verified.json', 256*1024)
    value = {'schema': 2, 'kind': 'couch-wifi-debug-transition-completed', 'complete': True,
        'restart_acknowledged': True, 'verified_receipt_sha256': recovery.sha(raw),
        'verification': json.loads(raw)}
    for name in ('restart-requested.json', 'restart-acknowledged.json'):
        value[name + '_sha256'] = recovery.sha(recovery.read(output/name, 4096))
    recovery.publish(output, 'completed.json', value)
    return recovery.sha(recovery.read(output/'completed.json', 256*1024))


def transition(proof, reader, writer_factory, output, *, bus, ports):
    """Write/readback boot once on the supplied same-connection writer facade.

    All original files remain untouched. No live partition is captured into a
    new baseline. A failed or ambiguous write is never retried.
    """
    checked = admit(proof.retained.source,
                    temporary_boot_sha256=proof.retained.temporary_boot_sha256,
                    original_boot_sha256=proof.original_boot_sha256,
                    snapshot_sha256=proof.snapshot_sha256,
                    image=proof.image, image_sha256=proof.image_sha256,
                    metadata=proof.metadata, metadata_sha256=proof.metadata_sha256)
    require(checked == proof, 'Transition evidence changed after admission')
    record = proof.retained.record
    evidence = receipt_evidence(proof, bus, ports)
    observed = copy.deepcopy(reader.description)
    require(observed.get('hwcode') == 0x6580
            and observed.get('cid_encoding') == 'mt6580-legacy-le32-registers'
            and observed['runtime_cid_sha256'] == recovery.sha(bytes.fromhex(record['cid']))
            and observed['capacity'] == record['capacity']
            and observed['partitions'] == record['partitions'],
            'Live identity/layout differs from retained failed session')
    retained = {name: item['sha256'] for name, item in record['originals'].items() if name != 'boot'}

    def check_live(device, boot):
        # The writer adds its model binding and changes the transport label;
        # compare hardware identity, not those facade-specific annotations.
        require(all(device.description.get(key) == observed[key] for key in
                    ('hwcode', 'cid_encoding', 'runtime_cid_sha256', 'capacity', 'partitions', 'storage_id')),
                'Transition connection identity changed')
        require(device.hash('boot') == boot, 'Current boot differs; no automatic transition retry')
        for name, expected in sorted(retained.items()):
            require(device.hash(name) == expected, 'Retained partition changed')

    check_live(reader, proof.retained.temporary_boot_sha256)
    output = Path(output).absolute()
    recovery.private_directory(output.parent)
    output.mkdir(mode=0o700)
    recovery.publish(output, 'admitted.json', {
        'schema': 1, 'kind': 'couch-wifi-debug-transition', 'complete': False,
        'source_session': str(proof.retained.source),
        'snapshot_sha256': proof.snapshot_sha256,
        'source_evidence_sha256': proof.retained.evidence_sha256,
        'original_boot_sha256': proof.original_boot_sha256,
        'previous_boot_sha256': proof.retained.temporary_boot_sha256,
        'debug_boot_sha256': proof.image_sha256,
        'debug_metadata_sha256': proof.metadata_sha256, 'write_targets': ['boot'],
    })
    binding = {**{k: observed[k] for k in ('storage_id', 'runtime_cid_sha256', 'partitions')},
               'model': MODEL, 'model_verified': True,
               'identity_sha256': fingerprint(record['identity_sha256'])}
    image = {'file': proof.image.name, 'sha256': proof.image_sha256,
             'size': record['partitions']['boot']['size']}
    release = {'schema': 1, 'model': MODEL, 'partitions': record['partitions'], 'images': {'boot': image}}
    writer = writer_factory(release, proof.image.parent, binding)
    try:
        check_live(writer, proof.retained.temporary_boot_sha256)
        writer.write('boot', proof.image)
        check_live(writer, proof.image_sha256)
        recovery.publish(output, 'verified.json', {
            'schema': 2, 'kind': 'couch-wifi-debug-transition', 'complete': True, **evidence,
            'written': ['boot'], 'boot_sha256': proof.image_sha256,
            'boot_readback_sha256': proof.image_sha256,
            'retained_before_sha256': retained, 'retained_sha256': retained, 'restart_requested': False,
        })
    finally:
        writer.close()
    return {'written': ['boot'], 'boot_sha256': proof.image_sha256,
            'verified': True, 'restart_requested': False}


def complete_chained_receipt(output):
    """Publish completion only after the chained boot request is acknowledged."""
    output = Path(output)
    raw = recovery.read(output/'verified.json', 256*1024)
    value = {'schema': 3, 'kind': 'couch-wifi-debug-chained-transition-completed', 'complete': True,
        'restart_acknowledged': True, 'verified_receipt_sha256': recovery.sha(raw),
        'verification': json.loads(raw)}
    for name in ('restart-requested.json', 'restart-acknowledged.json'):
        value[name + '_sha256'] = recovery.sha(recovery.read(output/name, 4096))
    recovery.publish(output, 'completed.json', value)
    return recovery.sha(recovery.read(output/'completed.json', 256*1024))


def transition_chained(proof, reader, writer_factory, output, *, bus, ports):
    """One b13-to-new-debug boot update; no recovery/session evidence is rewritten."""
    checked = admit_chained(proof.config, bus, ports)
    require(checked == proof, 'Chained transition evidence changed after admission')
    record = proof.parent.retained.record
    evidence = chained_receipt_evidence(proof, bus, ports)
    observed = copy.deepcopy(reader.description)
    require(observed.get('hwcode') == 0x6580
            and observed.get('cid_encoding') == 'mt6580-legacy-le32-registers'
            and observed['runtime_cid_sha256'] == recovery.sha(bytes.fromhex(record['cid']))
            and observed['capacity'] == record['capacity']
            and observed['partitions'] == record['partitions'],
            'Live identity/layout differs from retained parent session')
    retained = {name: item['sha256'] for name, item in record['originals'].items() if name != 'boot'}

    def check_live(device, boot):
        require(all(device.description.get(key) == observed[key] for key in
                    ('hwcode', 'cid_encoding', 'runtime_cid_sha256', 'capacity', 'partitions', 'storage_id')),
                'Chained transition connection identity changed')
        require(device.hash('boot') == boot, 'Current debug boot differs; no automatic transition retry')
        for name, expected in sorted(retained.items()):
            require(device.hash(name) == expected, 'Retained partition changed')

    check_live(reader, proof.previous_boot_sha256)
    output = Path(output).absolute()
    recovery.private_directory(output.parent)
    output.mkdir(mode=0o700)
    recovery.publish(output, 'admitted.json', {'schema': 3, 'kind': 'couch-wifi-debug-chained-transition',
        'complete': False, **chained_receipt_evidence(proof, bus, ports), 'write_targets': ['boot']})
    binding = {**{k: observed[k] for k in ('storage_id', 'runtime_cid_sha256', 'partitions')},
               'model': MODEL, 'model_verified': True,
               'identity_sha256': fingerprint(record['identity_sha256'])}
    image_record = {'file': proof.image.name, 'sha256': proof.image_sha256,
                    'size': record['partitions']['boot']['size']}
    release = {'schema': 1, 'model': MODEL, 'partitions': record['partitions'], 'images': {'boot': image_record}}
    writer = writer_factory(release, proof.image.parent, binding)
    try:
        check_live(writer, proof.previous_boot_sha256)
        writer.write('boot', proof.image)
        check_live(writer, proof.image_sha256)
        recovery.publish(output, 'verified.json', {
            'schema': 3, 'kind': 'couch-wifi-debug-chained-transition', 'complete': True, **evidence,
            'written': ['boot'], 'boot_sha256': proof.image_sha256,
            'boot_readback_sha256': proof.image_sha256,
            'retained_before_sha256': retained, 'retained_sha256': retained, 'restart_requested': False})
    finally:
        writer.close()
    return {'written': ['boot'], 'boot_sha256': proof.image_sha256,
            'verified': True, 'restart_requested': False}
