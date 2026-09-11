import json
import unittest
from types import SimpleNamespace
from couch_install import InstallError
from wifi_debug_worker import DebugWorker, DebugStageUsb, CAPABILITY

def debug_status(**extra):
    return {'status': 'ready', 'wifi_debug': True, 'capabilities': CAPABILITY,
        'stage_kind': 'private-ram-wifi-debug-stage', 'debug_protocol': 1, 'scan': False,
        'provisioned': False, 'debug_generation_limit': 8, **extra}

def diagnostic(status, **extra):
    return {'stage_kind': 'private-ram-wifi-debug-stage', 'capability': CAPABILITY,
        'debug_protocol': 1, 'precredential': True, 'status': status, 'log': '',
        'generation': 1, 'debug_generation_limit': 8,
        'lifecycle': {'supervisor': 'couch-wifi-debug-supervisor-v1', 'alive': True,
            'phase': 'worker-running', 'generation': 1, 'retry': 'none',
            'marker': False, 'worker': 'running', 'worker_exit': 'none'}, **extra}


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
        stage.read = lambda size: struct.pack('<4sIQ', b'CBR1', 0, 4609)
        with self.assertRaises(InstallError): stage.debug_request(8)
        stage.read = lambda size: struct.pack('<4sIQ', b'CBR1', 0, 4608) if size == 16 else b'x' * size
        self.assertEqual(len(stage.debug_request(8)), 4608)
        stage.read = lambda size: struct.pack('<4sIQ', b'CBR1', 0, 1)
        with self.assertRaises(InstallError): stage.debug_request(9)

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
        status = debug_status()
        worker.stage.dispatch = lambda *args: status
        worker.stage.debug_request = lambda opcode: calls.append(('opcode', opcode)) or (json.dumps(diagnostic(status)).encode() if opcode == 8 else b'')
        self.assertEqual(worker.dispatch({'op': 'debug_retry', 'payload': None}), {'accepted': True})
        self.assertEqual(calls[-1], ('opcode', 9))
        with self.assertRaises(InstallError):
            worker.dispatch({'op': 'debug_retry', 'payload': {'command': 'anything'}})

    def test_diagnostics_reject_non_precredential_record(self):
        worker, calls = self.fixture()
        self.open(worker)
        worker.stage.dispatch = lambda *args: debug_status()
        worker.stage.debug_request = lambda opcode: json.dumps({'precredential': False}).encode()
        with self.assertRaises(InstallError):
            worker.dispatch({'op': 'debug_status', 'payload': None})

    def test_diagnostics_require_exact_versioned_debug_schema(self):
        status = debug_status()
        valid = diagnostic(status)
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

    def test_diagnostics_require_bounded_typed_lifecycle(self):
        status = debug_status()
        for lifecycle, accepted in (
            (diagnostic(status)['lifecycle'], True),
            ({'supervisor': 'unexpected', 'alive': True, 'phase': 'worker-running',
              'generation': 1, 'retry': 'none', 'marker': False, 'worker': 'running',
              'worker_exit': 'none'}, False),
            ({'supervisor': 'couch-wifi-debug-supervisor-v1', 'alive': True,
              'phase': 'worker-running', 'generation': 9, 'retry': 'none', 'marker': False,
              'worker': 'running', 'worker_exit': 'none'}, False),
            ({'supervisor': 'couch-wifi-debug-supervisor-v1', 'alive': True,
              'phase': 'worker-running', 'generation': 1, 'retry': 'none', 'marker': False,
              'worker': 'running', 'worker_exit': 'none', 'private': 'text'}, False),
        ):
            worker, _ = self.fixture()
            self.open(worker)
            worker.stage.dispatch = lambda *args: status
            worker.stage.debug_request = lambda opcode: json.dumps(
                diagnostic(status, lifecycle=lifecycle)).encode()
            if accepted:
                self.assertEqual(worker.dispatch({'op': 'debug_status', 'payload': None})['lifecycle'], lifecycle)
            else:
                with self.assertRaises(InstallError):
                    worker.dispatch({'op': 'debug_status', 'payload': None})

    def test_diagnostic_log_bound_counts_utf8_bytes(self):
        status = debug_status()
        for log, accepted in (('é' * 2048, True), ('é' * 2048 + 'x', False), ('x' * 4097, False)):
            worker, calls = self.fixture()
            self.open(worker)
            worker.stage.dispatch = lambda *args: status
            response = diagnostic(status, log=log)
            worker.stage.debug_request = lambda opcode: json.dumps(response).encode()
            if accepted:
                self.assertEqual(worker.dispatch({'op': 'debug_status', 'payload': None})['log'], log)
            else:
                with self.assertRaises(InstallError): worker.dispatch({'op': 'debug_status', 'payload': None})

    def test_eighth_generation_never_sends_retry(self):
        for generation, error in ((8, 'none'), (7, 'debug-retry-limit'), (9, 'none')):
            worker, calls = self.fixture()
            self.open(worker)
            status = debug_status(error=error)
            worker.stage.dispatch = lambda *args: status
            def request(opcode):
                self.assertEqual(opcode, 8)
                return json.dumps(diagnostic(status, generation=generation)).encode()
            worker.stage.debug_request = request
            with self.assertRaises(InstallError): worker.dispatch({'op':'debug_retry', 'payload':None})

    def test_missing_or_wrong_op5_fields_never_send_op8_or_op9(self):
        invalid = []
        for key in ('stage_kind', 'debug_protocol', 'scan'):
            invalid.append({k: v for k, v in debug_status().items() if k != key})
        for key, value in (('stage_kind', 'private-install'), ('debug_protocol', True),
                           ('debug_protocol', 1.0), ('debug_protocol', 2), ('scan', True), ('scan', 0)):
            invalid.append(debug_status(**{key: value}))
        for status in invalid:
            for op in ('debug_status', 'debug_retry'):
                worker, calls = self.fixture()
                self.open(worker)
                sent = []
                worker.stage.dispatch = lambda *args: status
                worker.stage.debug_request = lambda opcode: sent.append(opcode)
                with self.assertRaises(InstallError): worker.dispatch({'op': op, 'payload': None})
                self.assertEqual(sent, [], (status, op))


if __name__ == '__main__':
    unittest.main()
