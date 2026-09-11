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


@dataclass(frozen=True)
class TransitionProof:
    retained: recovery.Proof
    original_boot_sha256: str
    snapshot_sha256: str
    image: Path
    image_sha256: str
    metadata: Path
    metadata_sha256: str


def admit(source, *, temporary_boot_sha256, original_boot_sha256,
          snapshot_sha256, image, image_sha256, metadata, metadata_sha256):
    for pin in (original_boot_sha256, snapshot_sha256, image_sha256, metadata_sha256):
        require(recovery.valid_hash(pin), 'Explicit transition evidence hash required')
    retained = recovery.admit(source, temporary_boot_sha256)
    require(retained.snapshot_sha256 == snapshot_sha256, 'Retained original snapshot differs from pin')
    require(retained.record['originals']['boot']['sha256'] == original_boot_sha256,
            'Original boot differs from retained baseline pin')
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
    require(image_sha256 not in (original_boot_sha256, temporary_boot_sha256),
            'Debug transition must use a distinct debug image')
    data = recovery.read(image, 16*1024*1024)
    require(len(data) == manifest['size'] == retained.record['partitions']['boot']['size']
            and data.startswith(b'ANDROID!') and recovery.sha(data) == image_sha256,
            'Debug image size/header/hash differs')
    return TransitionProof(retained, original_boot_sha256, snapshot_sha256,
                           image, image_sha256, metadata, metadata_sha256)


def transition(proof, reader, writer_factory, output):
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
            'schema': 1, 'kind': 'couch-wifi-debug-transition', 'complete': True,
            'written': ['boot'], 'boot_sha256': proof.image_sha256,
            'retained_sha256': retained, 'original_boot_sha256': proof.original_boot_sha256,
            'snapshot_sha256': proof.snapshot_sha256, 'restart_requested': False,
        })
    finally:
        writer.close()
    return {'written': ['boot'], 'boot_sha256': proof.image_sha256,
            'verified': True, 'restart_requested': False}
