"""Real stdio entrypoint with in-memory USB only; also used by native tests."""
import io
import json
from pathlib import Path
import runpy
import struct
import subprocess
import sys
import types
import unittest


def fixture(case):
    from test_wifi_debug_worker import debug_status, diagnostic
    status = debug_status(status='initializing', error='none')
    if case == 'identity':
        status['scan'] = True
    record = diagnostic(status, generation=2, step='power')
    body = b'{' if case == 'json' else json.dumps(record).encode()

    class USBError(OSError):
        backend_error_code = -4

    class FakeStageUsb:
        def __init__(self, *args):
            self.outgoing = types.SimpleNamespace(write=lambda data, **kw: len(data))
            self.response = io.BytesIO(struct.pack('<4sIQ', b'CBR1', 0, len(body)) + body)

        def dispatch(self, *args):
            if case in ('transport', 'cleanup'):
                raise USBError(19, '/private/credential-SECRET')
            return status

        def read(self, size):
            return self.response.read(size)

        def close(self):
            if case == 'cleanup':
                raise RuntimeError('cleanup-SECRET')

    stage = types.ModuleType('stage_usb')
    stage.StageUsb = FakeStageUsb
    sys.modules['stage_usb'] = stage
    # The real entrypoint runs, but importing/accessing real USB is impossible.
    for name in ('usb', 'usb.core', 'usb.util', 'usb.backend', 'usb.backend.libusb1'):
        sys.modules[name] = types.ModuleType(name)
    usb = sys.modules['usb']
    usb.core, usb.util, usb.backend = (sys.modules[n] for n in ('usb.core', 'usb.util', 'usb.backend'))
    usb.backend.libusb1 = sys.modules['usb.backend.libusb1']
    usb.backend.libusb1.get_backend = lambda **kw: object()
    runpy.run_path(str(Path(__file__).with_name('wifi_debug_worker.py')), run_name='__main__')


def frame(value):
    data = json.dumps(value).encode()
    return struct.pack('<I', len(data)) + data


class BoundaryTests(unittest.TestCase):
    def test_real_entrypoint_reports_safe_failure_or_valid_generation_two(self):
        commands = [
            {'op': 'debug_open', 'payload': {'bus': 1, 'ports': [1], 'libusb': '/fake', 'wait_seconds': 0}},
            {'op': 'debug_status', 'payload': None},
            {'op': 'debug_close', 'payload': None},
        ]
        for case, category in [('valid', None), ('transport', 'USBError'),
                               ('cleanup', 'USBError'), ('identity', 'ContractError'), ('json', 'DecodeError')]:
            with self.subTest(case=case):
                result = subprocess.run([sys.executable, '-B', __file__, '--fixture', case],
                    input=b''.join(map(frame, commands)), capture_output=True, timeout=10)
                data, events = result.stdout, []
                self.assertEqual(result.stderr, b'')
                self.assertNotIn(b'SECRET', data)
                while data:
                    self.assertGreaterEqual(len(data), 4)
                    size, = struct.unpack('<I', data[:4])
                    self.assertGreaterEqual(len(data), size + 4)
                    event = json.loads(data[4:4 + size])
                    if event['event'] == 'debug_error':
                        self.assertLessEqual(size, 512)
                    events.append(event)
                    data = data[4 + size:]
                if category is None:
                    self.assertEqual(result.returncode, 0)
                    self.assertEqual([v['event'] for v in events], ['debug_open', 'debug_status', 'debug_close'])
                    self.assertEqual(events[1]['result']['generation'], 2)
                else:
                    self.assertEqual(result.returncode, 1)
                    self.assertEqual([v['event'] for v in events], ['debug_open', 'debug_error'])
                    failure = events[1]['diagnostic']
                    self.assertEqual(failure['operation'], 'debug_status')
                    self.assertEqual(failure['category'], category)
                    if category == 'USBError':
                        self.assertEqual((failure['errno'], failure['backend_error_code']), (19, -4))

    def test_error_record_rejects_arbitrary_text_and_numeric_values(self):
        from wifi_debug_worker import failure_record
        error = RuntimeError('SECRET')
        error.errno, error.backend_error_code = True, 65537
        value = failure_record(error, 'SECRET', 'request')
        self.assertEqual(value, {'operation': 'unknown', 'phase': 'request', 'category': 'WorkerError'})


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--fixture':
        fixture(sys.argv[2])
    else:
        unittest.main()
