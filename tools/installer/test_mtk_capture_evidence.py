"""Worker-owned capture proof: no physical device or host-asserted verification."""
import copy
import hashlib
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest

from couch_install import IDENTITY_PARTITIONS, InstallError
from mtk_adapter import Adapter, READABLE


class CaptureTests(unittest.TestCase):
    def fixture(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        stage = Path(directory.name)/'stage.img'
        with stage.open('wb') as output:
            output.write(b'ANDROID!')
            output.truncate(16*1024*1024)
        self.counts = {name: 0 for name in READABLE}
        self.postchecks, self.writes, self.writer_calls = [], [], []
        self.hashes = {}
        desc = {'runtime_cid_sha256': '1'*64, 'partitions': {}}
        offset = 0
        for name in sorted(READABLE):
            size = 16*1024*1024 if name == 'boot' else 512
            desc['partitions'][name] = {'offset': offset, 'size': size}
            offset += size
            self.hashes[name] = hashlib.sha256(b'\0'*size).hexdigest()
        def chunks(name):
            self.counts[name] += 1
            size = desc['partitions'][name]['size']
            for index in range(0, size, 1024*1024):
                yield b'\0'*min(1024*1024, size-index)
        reader = SimpleNamespace(description=desc, runtime_cid='12'*16, chunks=chunks)
        backend = SimpleNamespace(device=object(), mtk=object(), selected=None, close=lambda **kw: None)
        backend.claim = lambda selected: setattr(backend, 'selected', selected)
        backend.claimed_candidate = lambda: backend.selected
        backend.start_readonly = lambda *_: backend.mtk
        def posthash(name):
            self.postchecks.append(name)
            return '2'*64 if name == 'boot' else self.hashes[name]
        def writer(*args, **kwargs):
            self.writer_calls.append(True)
            return SimpleNamespace(write=lambda *args: self.writes.append(args), hash=posthash, close=lambda: None)
        wire = SimpleNamespace(send=lambda value: None, chunk=lambda value: None)
        adapter = Adapter(wire, reader=lambda *_: reader, writer=writer)
        adapter.backend, adapter.loader = backend, b'loader'
        adapter.dispatch({'op': 'start', 'candidate': {'bus': 1, 'address': 2, 'ports': [3], 'vid': 0xe8d, 'pid': 0x2000}})
        command = {'op': 'authorize_boot', 'cid_sha256': desc['runtime_cid_sha256'],
                   'partitions': copy.deepcopy(desc['partitions']),
                   'identity_sha256': {name: self.hashes[name] for name in IDENTITY_PARTITIONS},
                   'original_sha256': {name: self.hashes[name] for name in READABLE-IDENTITY_PARTITIONS},
                   'stage': str(stage), 'stage_sha256': '2'*64}
        return adapter, command

    def capture(self, adapter, *, read=True, verify=True):
        for name in sorted(READABLE):
            if read:
                adapter.dispatch({'op': 'read', 'target': name})
            if verify:
                adapter.dispatch({'op': 'hash', 'target': name})

    def assert_denied(self, adapter, command):
        with self.assertRaises(InstallError):
            adapter.dispatch(command)
        with self.assertRaises(InstallError):
            adapter.dispatch(command)
        with self.assertRaises(InstallError):
            adapter.dispatch({'op': 'write_boot'})
        self.assertEqual(self.writer_calls, [])
        self.assertEqual(self.writes, [])
        self.assertFalse(adapter.capture_verified)

    def test_authorization_reuses_two_independent_reads_and_preserves_all_postchecks(self):
        adapter, command = self.fixture()
        self.capture(adapter)
        self.assertEqual(self.counts, dict.fromkeys(READABLE, 2))
        adapter.dispatch(command)
        self.assertEqual(self.counts, dict.fromkeys(READABLE, 2))
        self.assertEqual(self.writer_calls, [True])
        self.assertFalse(adapter.capture_verified)
        adapter.dispatch({'op': 'write_boot'})
        self.assertEqual(len(self.writes), 1)
        self.assertEqual(set(self.postchecks), READABLE)
        self.assertEqual(len(self.postchecks), 9)
        self.assertTrue(adapter.verified)
        with self.assertRaises(InstallError):
            adapter.dispatch(command)
        with self.assertRaises(InstallError):
            adapter.dispatch({'op': 'write_boot'})
        self.assertEqual(len(self.writes), 1)

    def test_host_hashes_or_only_one_read_cannot_supply_worker_proof(self):
        for read, verify in [(False, False), (True, False), (False, True)]:
            with self.subTest(read=read, verify=verify):
                adapter, command = self.fixture()
                self.capture(adapter, read=read, verify=verify)
                self.assert_denied(adapter, command)
        adapter, command = self.fixture()
        self.capture(adapter)
        command['original_sha256']['boot'] = 'f'*64
        self.assert_denied(adapter, command)

    def test_changed_connection_cid_layout_or_reader_invalidates_proof(self):
        for change in ['device', 'backend', 'mtk', 'reader', 'selection', 'cid', 'layout', 'runtime_cid']:
            with self.subTest(change=change):
                adapter, command = self.fixture()
                self.capture(adapter)
                if change in ('device', 'mtk'):
                    setattr(adapter.backend, change, object())
                elif change == 'backend':
                    adapter.backend = copy.copy(adapter.backend)
                elif change == 'reader':
                    adapter.reader = copy.copy(adapter.reader)
                elif change == 'selection':
                    adapter.selected = None
                elif change == 'cid':
                    adapter.reader.description['runtime_cid_sha256'] = '9'*64
                elif change == 'layout':
                    adapter.reader.description['partitions']['boot']['offset'] += 512
                else:
                    adapter.reader.runtime_cid = '34'*16
                self.assert_denied(adapter, command)
                self.assertEqual(self.counts, dict.fromkeys(READABLE, 2))

    def test_repeat_capture_requires_new_independent_verification(self):
        adapter, command = self.fixture()
        self.capture(adapter)
        adapter.dispatch({'op': 'read', 'target': 'boot'})
        self.assert_denied(adapter, command)

    def test_short_read_mismatch_or_failed_delivery_poison_capture(self):
        for failure in ('short', 'mismatch', 'delivery'):
            with self.subTest(failure=failure):
                adapter, command = self.fixture()
                adapter.dispatch({'op': 'read', 'target': 'boot'})
                if failure == 'short':
                    adapter.reader.chunks = lambda name: iter([b'\0'*512])
                    op = 'read'
                elif failure == 'mismatch':
                    adapter.reader.chunks = lambda name: iter([b'1'*(1024*1024)]*16)
                    op = 'hash'
                else:
                    def send(value):
                        if value['event'] == 'hash':
                            raise OSError('fixture disconnect')
                    adapter.wire.send = send
                    op = 'hash'
                with self.assertRaises((InstallError, OSError)):
                    adapter.dispatch({'op': op, 'target': 'boot'})
                self.assert_denied(adapter, command)

    def test_close_cannot_resume_verified_capture(self):
        adapter, command = self.fixture()
        self.capture(adapter)
        adapter.close()
        self.assert_denied(adapter, command)


if __name__ == '__main__':
    unittest.main()
