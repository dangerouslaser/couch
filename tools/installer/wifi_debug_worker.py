"""Attach-only USB bridge for the native Wi-Fi debugger.

No MTK, serial, enrollment, boot, binding, or storage operations. The native
parent holds the normal installer USB lease and supervises every request.
"""
import json
from pathlib import Path
import struct
import sys
import time
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
from couch_install import InstallError, require
from stage_usb import StageUsb

MAX = 32768
MAX_DIAGNOSTIC = 4608
MAX_LOG = 4096
CAPABILITY = 'COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1'
OPERATIONS = ('debug_open', 'stage_status', 'debug_status', 'debug_retry', 'debug_close')


def failure_record(error, operation, phase):
    # Never serialize exception text, arguments, paths, payloads or tracebacks.
    category = type(error).__name__
    if isinstance(error, InstallError):
        category = 'ContractError'
    elif isinstance(error, (json.JSONDecodeError, UnicodeDecodeError)):
        category = 'DecodeError'
    elif category not in ('USBError', 'USBTimeoutError', 'OSError', 'TimeoutError'):
        category = 'WorkerError'
    result = {'operation': operation if operation in OPERATIONS else 'unknown',
              'phase': phase, 'category': category}
    for name in ('errno', 'backend_error_code'):
        value = getattr(error, name, None)
        if type(value) is int and -65536 <= value <= 65536:
            result[name] = value
    return result


def debug_identity(status):
    return (isinstance(status, dict) and status.get('wifi_debug') is True
            and status.get('capabilities') == CAPABILITY
            and status.get('stage_kind') == 'private-ram-wifi-debug-stage'
            and type(status.get('debug_protocol')) is int and status['debug_protocol'] == 1
            and status.get('scan') is False
            and type(status.get('debug_generation_limit')) is int and status['debug_generation_limit'] == 8
            and status.get('provisioned') is False)


class DebugStageUsb(StageUsb):
    def debug_request(self, opcode):
        require(opcode in (8, 9), 'Unsupported debug opcode')
        # Base transport deliberately excludes debug opcodes. Keep the separate
        # protocol here so the normal installer retains its existing allowlist.
        header = struct.pack('<4sIQ', b'CBP1', opcode, 0)
        require(self.outgoing.write(header, timeout=30000) == 16, 'Short debug request')
        magic, status, size = struct.unpack('<4sIQ', self.read(16))
        require(magic == b'CBR1' and status == 0 and size <= (MAX_DIAGNOSTIC if opcode == 8 else 0),
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
        self.failure = None

    def dispatch(self, command):
        require(not self.failed, 'Debug connection stopped')
        op, phase = 'unknown', 'request'
        try:
            require(isinstance(command, dict) and set(command) == {'op', 'payload'},
                    'Invalid debug command')
            op, payload = command['op'], command['payload']
            if op == 'debug_open':
                require(self.stage is None and isinstance(payload, dict)
                        and set(payload) == {'bus', 'ports', 'libusb', 'wait_seconds'}, 'Invalid debug attachment')
                require(type(payload['wait_seconds']) is int and payload['wait_seconds'] in (0, 60),
                        'Invalid stage enumeration wait')
                require(type(payload['bus']) is int and 0 < payload['bus'] <= 255
                        and isinstance(payload['ports'], list) and 1 <= len(payload['ports']) <= 7
                        and all(type(n) is int and 0 < n <= 255 for n in payload['ports']),
                        'Invalid physical USB port')
                phase = 'attach'
                self.stage = self.factory(payload)
                return {'attached': True}
            require(self.stage is not None, 'Debug stage is not attached')
            require(op in ('stage_status', 'debug_status', 'debug_retry', 'debug_close'),
                    'Unsupported debug operation')
            if op == 'debug_close':
                require(payload is None, 'Unexpected close payload')
                phase = 'close'
                self.close()
                return {'closed': True}
            require(payload is None, 'Debug operations take no payload')
            if op in ('debug_status', 'debug_retry'):
                phase = 'status'
                status = self.stage.dispatch('stage_status', None)
                phase = 'identity'
                require(debug_identity(status),
                        'Expected unprovisioned dedicated Wi-Fi debug stage')
                # Refresh bounded diagnostic generation immediately before a
                # retry; never send op9 once the stage exhausted its eight runs.
                phase = 'diagnostic-read'
                raw = self.stage.debug_request(8)
                phase = 'diagnostic-parse'
                value = json.loads(raw)
                phase = 'diagnostic-contract'
                require(isinstance(value, dict)
                        and value.get('stage_kind') == 'private-ram-wifi-debug-stage'
                        and value.get('capability') == CAPABILITY
                        and type(value.get('debug_protocol')) is int and value['debug_protocol'] == 1
                        and value.get('precredential') is True
                        and debug_identity(value.get('status')),
                        'Expected versioned pre-credential debug diagnostic record')
                require(type(value.get('debug_generation_limit')) is int and value['debug_generation_limit'] == 8
                        and type(value.get('generation')) is int and 0 <= value['generation'] <= 8,
                        'Invalid debug generation bound')
                require(isinstance(value.get('log'), str)
                        and len(value['log'].encode('utf-8')) <= MAX_LOG,
                        'Diagnostic log exceeds UTF-8 byte bound')
                if op == 'debug_retry':
                    require(value['generation'] < 8 and value['status'].get('error') != 'debug-retry-limit',
                            'Debug retry limit reached')
                    phase = 'retry'
                    require(self.stage.debug_request(9) == b'', 'Unexpected retry acknowledgement')
                    return {'accepted': True}
                return value
            phase = 'status'
            return self.stage.dispatch(op, payload)
        except BaseException as error:
            # Preserve the first failure even if releasing a disconnected USB
            # interface fails too. A failed connection is never reused.
            self.failure = failure_record(error, op, phase)
            self.failed = True
            try:
                self.close()
            except BaseException:
                pass
            raise

    def close(self):
        if self.stage is not None:
            stage = self.stage
            self.stage = None
            stage.close()


def attach(payload):
    # Import PyUSB only after an explicit, validated attach request.
    import usb.core
    import usb.util
    import usb.backend.libusb1
    backend = usb.backend.libusb1.get_backend(find_library=lambda _: payload['libusb'])
    require(backend is not None, 'Pinned libusb is unavailable')
    deadline = time.monotonic() + payload['wait_seconds']
    while payload['wait_seconds']:
        matches = [d for d in usb.core.find(find_all=True, idVendor=0x0e8d, idProduct=0x201c, backend=backend)
                   if d.bus == payload['bus'] and tuple(d.port_numbers or ()) == tuple(payload['ports'])]
        require(len(matches) <= 1, 'Multiple stages match selected physical port')
        if matches:
            break
        require(time.monotonic() < deadline, 'Debug stage did not appear after transition')
        time.sleep(0.2)
    return DebugStageUsb(usb, backend, SimpleNamespace(bus=payload['bus'], ports=tuple(payload['ports'])))


def main():
    worker = DebugWorker(attach)
    operation, phase, response_started = 'unknown', 'request', False
    try:
        while True:
            operation, phase, response_started = 'unknown', 'request', False
            size, = struct.unpack('<I', exact(sys.stdin.buffer, 4))
            require(0 < size <= MAX, 'Debug request exceeds bound')
            command = json.loads(exact(sys.stdin.buffer, size))
            if isinstance(command, dict) and command.get('op') in OPERATIONS:
                operation = command['op']
            result = worker.dispatch(command)
            phase = 'response'
            data = json.dumps({'event': command['op'], 'result': result}, separators=(',', ':')).encode()
            require(len(data) <= MAX, 'Debug response exceeds bound')
            response_started = True
            sys.stdout.buffer.write(struct.pack('<I', len(data)) + data)
            sys.stdout.buffer.flush()
            if command['op'] == 'debug_close':
                return
    except Exception as error:
        # A partially written response cannot safely be followed by another
        # frame. Otherwise report the bounded failure before closing the pipe.
        if not response_started:
            failure = worker.failure or failure_record(error, operation, phase)
            data = json.dumps({'event': 'debug_error', 'diagnostic': failure},
                              separators=(',', ':')).encode()
            try:
                require(len(data) <= 512, 'Debug error exceeds bound')
                sys.stdout.buffer.write(struct.pack('<I', len(data)) + data)
                sys.stdout.buffer.flush()
            except Exception:
                pass
        raise
    finally:
        try:
            worker.close()
        except BaseException:
            pass


if __name__ == '__main__':
    try:
        main()
    except Exception:
        # Safe detail uses the framed native channel, never raw stderr.
        sys.exit(1)
