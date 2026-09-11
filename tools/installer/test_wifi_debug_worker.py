import json
import unittest
from types import SimpleNamespace
from couch_install import InstallError
from wifi_debug_worker import DebugWorker, DebugStageUsb, CAPABILITY


class DebugTests(unittest.TestCase):
    def test_debug_frames_are_bounded_and_retry_has_no_body(self):
        import struct
        stage = DebugStageUsb.__new__(DebugStageUsb)
        writes = []
        stage.outgoing = SimpleNamespace(write=lambda data, **kw: writes.append(data) or len(data))
        stage.read = lambda size: struct.pack('<4sIQ', b'CBR1', 0, 0) if size == 16 else b''
        self.assertEqual(stage.debug_request(9), b'')
        self.assertEqual(writes, [struct.pack('<4sIQ', b'CBP1', 9, 0)])
        for opcode in (3, 4, 6, 10):
            with self.assertRaises(InstallError): stage.debug_request(opcode)
        self.assertEqual(len(writes), 1)
        stage.read = lambda size: struct.pack('<4sIQ', b'CBR1', 0, 32769)
        with self.assertRaises(InstallError): stage.debug_request(8)

    def fixture(self):
        calls = []
        stage = SimpleNamespace(dispatch=lambda op, payload: calls.append((op, payload)) or {'status': 'ready'},
                                close=lambda: calls.append(('close', None)))
        worker = DebugWorker(lambda selection: calls.append(('open', selection)) or stage)
        return worker, calls

    def open(self, worker):
        worker.dispatch({'op': 'debug_open', 'payload': {'bus': 1, 'ports': [2], 'libusb': '/pinned/library', 'wait_seconds': 0}})

    def test_only_fixed_stage_operations_and_no_boot_or_binding(self):
        for forbidden in ('prepare', 'start', 'authorize_boot', 'boot', 'stage_bind', 'stage_provision', 'stage_scan', 'write', 'exec', 'restart'):
            worker, calls = self.fixture()
            self.open(worker)
            with self.assertRaises(InstallError):
                worker.dispatch({'op': forbidden, 'payload': None})
            self.assertEqual([c[0] for c in calls], ['open', 'close'])
            with self.assertRaises(InstallError):
                worker.dispatch({'op': 'stage_status', 'payload': None})

    def test_invalid_topology_never_opens_usb(self):
        for bus, ports in ((0, [1]), (True, [1]), (1, []), (1, [0]), (1, [True]), (1, [1] * 8)):
            worker, calls = self.fixture()
            with self.assertRaises(InstallError):
                worker.dispatch({'op': 'debug_open', 'payload': {'bus': bus, 'ports': ports, 'libusb': '/x', 'wait_seconds': 0}})
            self.assertEqual(calls, [])

    def test_status_and_close_use_one_retained_connection(self):
        worker, calls = self.fixture()
        self.open(worker)
        self.assertEqual(worker.dispatch({'op': 'stage_status', 'payload': None}), {'status': 'ready'})
        worker.dispatch({'op': 'debug_close', 'payload': None})
        self.assertEqual([c[0] for c in calls], ['open', 'stage_status', 'close'])

    def test_old_or_provisioned_stage_gets_no_unknown_debug_opcode(self):
        for status in ({'status': 'ready'}, {'wifi_debug': True, 'capabilities': CAPABILITY, 'provisioned': True},
                       {'wifi_debug': True, 'capabilities': 'wrong', 'provisioned': False}):
            for op in ('debug_status', 'debug_retry'):
                worker, calls = self.fixture()
                self.open(worker)
                worker.stage.dispatch = lambda *args: status
                worker.stage.debug_request = lambda opcode: self.fail('debug opcode sent before identity gate')
                with self.assertRaises(InstallError):
                    worker.dispatch({'op': op, 'payload': None})

    def test_retry_uses_only_empty_debug_opcode_after_identity(self):
        worker, calls = self.fixture()
        self.open(worker)
        worker.stage.dispatch = lambda *args: {'wifi_debug': True, 'capabilities': CAPABILITY, 'provisioned': False}
        worker.stage.debug_request = lambda opcode: calls.append(('opcode', opcode)) or b''
        self.assertEqual(worker.dispatch({'op': 'debug_retry', 'payload': None}), {'accepted': True})
        self.assertEqual(calls[-1], ('opcode', 9))
        with self.assertRaises(InstallError):
            worker.dispatch({'op': 'debug_retry', 'payload': {'command': 'anything'}})

    def test_diagnostics_reject_non_precredential_record(self):
        worker, calls = self.fixture()
        self.open(worker)
        worker.stage.dispatch = lambda *args: {'wifi_debug': True, 'capabilities': CAPABILITY, 'provisioned': False}
        worker.stage.debug_request = lambda opcode: json.dumps({'precredential': False}).encode()
        with self.assertRaises(InstallError):
            worker.dispatch({'op': 'debug_status', 'payload': None})

    def test_diagnostics_require_exact_versioned_debug_schema(self):
        status = {'status': 'ready', 'wifi_debug': True, 'capabilities': CAPABILITY, 'provisioned': False}
        valid = {'stage_kind': 'private-ram-wifi-debug-stage', 'capability': CAPABILITY,
                 'debug_protocol': 1, 'precredential': True, 'status': status}
        cases = [(valid, True)]
        for key in ('stage_kind', 'capability', 'debug_protocol', 'precredential'):
            cases.append(({k: v for k, v in valid.items() if k != key}, False))
        for key, value in (('stage_kind', 'benchmark'), ('capability', 'wrong'),
                           ('debug_protocol', True), ('debug_protocol', 2),
                           ('precredential', False), ('status', {'provisioned': False})):
            cases.append(({**valid, key: value}, False))
        for response, accepted in cases:
            worker, calls = self.fixture()
            self.open(worker)
            worker.stage.dispatch = lambda *args: status
            worker.stage.debug_request = lambda opcode: json.dumps(response).encode()
            if accepted:
                self.assertEqual(worker.dispatch({'op': 'debug_status', 'payload': None}), valid)
            else:
                with self.assertRaises(InstallError): worker.dispatch({'op': 'debug_status', 'payload': None})


if __name__ == '__main__':
    unittest.main()
