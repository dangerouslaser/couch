"""Attach-only USB bridge for the native Wi-Fi debugger.

No MTK, serial, enrollment, boot, binding, or storage operations. The native
parent holds the normal installer USB lease and supervises every request.
"""
import json
from pathlib import Path
import struct
import sys
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
from couch_install import require
from stage_usb import StageUsb

MAX = 32768
CAPABILITY = 'COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1'


def debug_identity(status):
    return (isinstance(status, dict) and status.get('wifi_debug') is True
            and status.get('capabilities') == CAPABILITY
            and status.get('provisioned') is False)


class DebugStageUsb(StageUsb):
    def debug_request(self, opcode):
        require(opcode in (8, 9), 'Unsupported debug opcode')
        # Base transport deliberately excludes debug opcodes. Keep the separate
        # protocol here so the normal installer retains its existing allowlist.
        header = struct.pack('<4sIQ', b'CBP1', opcode, 0)
        require(self.outgoing.write(header, timeout=30000) == 16, 'Short debug request')
        magic, status, size = struct.unpack('<4sIQ', self.read(16))
        require(magic == b'CBR1' and status == 0 and size <= (32768 if opcode == 8 else 0),
                'Invalid debug response')
        return self.read(size)


def exact(stream, size):
    data = bytearray()
    while len(data) < size:
        block = stream.read(size - len(data))
        require(block, 'Debug host disconnected')
        data.extend(block)
    return bytes(data)


class DebugWorker:
    def __init__(self, factory):
        self.factory = factory
        self.stage = None
        self.failed = False

    def dispatch(self, command):
        require(not self.failed, 'Debug connection stopped')
        try:
            require(isinstance(command, dict) and set(command) == {'op', 'payload'},
                    'Invalid debug command')
            op, payload = command['op'], command['payload']
            if op == 'debug_open':
                require(self.stage is None and isinstance(payload, dict)
                        and set(payload) == {'bus', 'ports', 'libusb'}, 'Invalid debug attachment')
                require(type(payload['bus']) is int and 0 < payload['bus'] <= 255
                        and isinstance(payload['ports'], list) and 1 <= len(payload['ports']) <= 7
                        and all(type(n) is int and 0 < n <= 255 for n in payload['ports']),
                        'Invalid physical USB port')
                self.stage = self.factory(payload)
                return {'attached': True}
            require(self.stage is not None, 'Debug stage is not attached')
            require(op in ('stage_status', 'debug_status', 'debug_retry', 'debug_close'),
                    'Unsupported debug operation')
            if op == 'debug_close':
                require(payload is None, 'Unexpected close payload')
                self.close()
                return {'closed': True}
            require(payload is None, 'Debug operations take no payload')
            if op in ('debug_status', 'debug_retry'):
                require(debug_identity(self.stage.dispatch('stage_status', None)),
                        'Expected unprovisioned dedicated Wi-Fi debug stage')
                raw = self.stage.debug_request(8 if op == 'debug_status' else 9)
                if op == 'debug_retry':
                    return {'accepted': True}
                value = json.loads(raw)
                require(isinstance(value, dict) and value.get('precredential') is True,
                        'Expected pre-credential diagnostic record')
                return value
            return self.stage.dispatch(op, payload)
        except BaseException:
            self.failed = True
            self.close()
            raise

    def close(self):
        if self.stage is not None:
            self.stage.close()
            self.stage = None


def attach(payload):
    # Import PyUSB only after an explicit, validated attach request.
    import usb.core
    import usb.util
    import usb.backend.libusb1
    backend = usb.backend.libusb1.get_backend(find_library=lambda _: payload['libusb'])
    require(backend is not None, 'Pinned libusb is unavailable')
    return DebugStageUsb(usb, backend, SimpleNamespace(bus=payload['bus'], ports=tuple(payload['ports'])))


def main():
    worker = DebugWorker(attach)
    try:
        while True:
            size, = struct.unpack('<I', exact(sys.stdin.buffer, 4))
            require(0 < size <= MAX, 'Debug request exceeds bound')
            command = json.loads(exact(sys.stdin.buffer, size))
            result = worker.dispatch(command)
            data = json.dumps({'event': command['op'], 'result': result}, separators=(',', ':')).encode()
            require(len(data) <= MAX, 'Debug response exceeds bound')
            sys.stdout.buffer.write(struct.pack('<I', len(data)) + data)
            sys.stdout.buffer.flush()
            if command['op'] == 'debug_close':
                return
    finally:
        worker.close()


if __name__ == '__main__':
    try:
        main()
    except Exception:
        # Exceptions can contain USB payloads. The supervisor reports a generic
        # failed operation; never copy exceptions or credentials to stderr.
        sys.exit(1)
