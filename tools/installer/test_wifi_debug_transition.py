import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from couch_install import InstallError
import recover_native_bootstrap as recovery
import wifi_debug_transition as debug


class TransitionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.root.chmod(0o700)
        self.source = self.root/'failed-session'
        self.source.mkdir(mode=0o700)
        self.image = self.root/'debug.img'
        self.image.write_bytes(b'ANDROID!' + bytes(16*1024*1024-8))
        self.image_hash = recovery.sha(self.image.read_bytes())
        self.record = {'cid': '1'*32, 'capacity': 128*1024*1024,
                       'partitions': {'boot': {'offset': 0, 'size': 16*1024*1024}},
                       'originals': {'boot': {'sha256': 'a'*64}, 'nvram': {'sha256': 'b'*64}},
                       'identity_sha256': {'nvram': 'b'*64}}
        self.retained = recovery.Proof(self.source, self.record, 'c'*64, 'd'*64, {'event': 'e'*64})
        self.metadata = self.root/'debug.json'
        self.metadata.write_text(json.dumps({'schema': 1, 'kind': 'private-ram-wifi-debug-stage',
            'private_only': True, 'installable': False, 'storage_operations': [],
            'sha256': self.image_hash, 'size': self.image.stat().st_size}))
        self.args = dict(temporary_boot_sha256='d'*64, original_boot_sha256='a'*64,
            snapshot_sha256='c'*64, image=self.image, image_sha256=self.image_hash,
            metadata=self.metadata, metadata_sha256=recovery.sha(self.metadata.read_bytes()))
        # The existing recovery tests exercise real nine-file/journal admission.
        # This fixture isolates the new image and transition gates.
        self.admission = patch.object(recovery, 'admit', return_value=self.retained)
        self.admission.start()
        self.addCleanup(self.admission.stop)

    def device(self):
        record = self.record
        class Device:
            def __init__(self):
                self.description = {'hwcode': 0x6580, 'cid_encoding': 'mt6580-legacy-le32-registers',
                    'runtime_cid_sha256': recovery.sha(bytes.fromhex(record['cid'])),
                    'capacity': record['capacity'], 'partitions': copy.deepcopy(record['partitions']),
                    'storage_id': 'f'*64}
                self.hashes = {'boot': 'd'*64, 'nvram': 'b'*64}
                self.writes = []
                self.closed = False
            def hash(self, name): return self.hashes[name]
            def write(self, name, path):
                self.writes.append(name)
                self.hashes[name] = recovery.sha(Path(path).read_bytes())
            def close(self): self.closed = True
        return Device()

    def test_one_boot_write_readback_keeps_original_baseline(self):
        proof = debug.admit(self.source, **self.args)
        device = self.device()
        writer = self.device()
        writer.description.update(transport='mtkclient-connected-private-writer',
                                  model_verified=True, identity_sha256='f'*64)
        def factory(release, bundle, binding):
            self.assertEqual(set(release['images']), {'boot'})
            self.assertEqual(release['images']['boot']['sha256'], self.image_hash)
            return writer
        result = debug.transition(proof, device, factory, self.root/'transition')
        self.assertEqual(writer.writes, ['boot'])
        self.assertTrue(result['verified'])
        self.assertFalse(result['restart_requested'])
        self.assertTrue(writer.closed)
        self.assertEqual(list(self.source.iterdir()), [])
        receipt = json.loads((self.root/'transition/verified.json').read_text())
        self.assertEqual(receipt['original_boot_sha256'], 'a'*64)

    def test_wrong_baseline_or_non_debug_image_fails_offline(self):
        for key in ('original_boot_sha256', 'snapshot_sha256', 'metadata_sha256', 'image_sha256'):
            args = {**self.args, key: '0'*64}
            with self.assertRaises(InstallError): debug.admit(self.source, **args)
        value = json.loads(self.metadata.read_text())
        value['kind'] = 'private-ram-wifi-installer'
        self.metadata.write_text(json.dumps(value))
        args = {**self.args, 'metadata_sha256': recovery.sha(self.metadata.read_bytes())}
        with self.assertRaises(InstallError): debug.admit(self.source, **args)

    def test_wrong_live_temp_or_retained_identity_never_constructs_writer(self):
        proof = debug.admit(self.source, **self.args)
        for name in ('boot', 'nvram', 'cid'):
            device = self.device()
            if name == 'cid': device.description['runtime_cid_sha256'] = '0'*64
            else: device.hashes[name] = '0'*64
            with self.assertRaises(InstallError):
                debug.transition(proof, device, lambda *args: self.fail('writer constructed'), self.root/name)
            self.assertEqual(device.writes, [])

    def test_bad_readback_has_no_success_receipt_or_retry(self):
        proof = debug.admit(self.source, **self.args)
        device = self.device()
        device.write = lambda name, path: device.writes.append(name)
        with self.assertRaises(InstallError):
            debug.transition(proof, device, lambda *args: device, self.root/'failed-transition')
        self.assertEqual(device.writes, ['boot'])
        self.assertFalse((self.root/'failed-transition/verified.json').exists())

    def test_changed_image_or_connection_never_writes(self):
        proof = debug.admit(self.source, **self.args)
        device = self.device()
        writer = self.device()
        writer.description['storage_id'] = '0'*64
        with self.assertRaises(InstallError):
            debug.transition(proof, device, lambda *args: writer, self.root/'changed-connection')
        self.assertEqual(writer.writes, [])
        self.image.write_bytes(b'changed')
        with self.assertRaises(InstallError):
            debug.transition(proof, device, lambda *args: self.fail('writer constructed'), self.root/'changed-image')
        self.assertEqual(device.writes, [])


if __name__ == '__main__': unittest.main()
