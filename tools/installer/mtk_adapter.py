#!/usr/bin/env python3
"""Private native-host MTK worker. Fixed RPC operations, no discovery or writes at startup.

Rust owns admission, durable originals, process locks/deadlines and sequencing.
This worker retains the reviewed MTK handshake/read/write implementation only.
Use an independently verified Python/runtime/source bundle. Never run it against
hardware outside the native host's admitted and supervised session.
"""
import hashlib
import json
from contextlib import contextmanager
from pathlib import Path
import struct
import sys

# -I deliberately omits the script directory. This file and its sibling modules
# are themselves part of the installer's verified source inventory.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from couch_install import IDENTITY_PARTITIONS, MODEL, fingerprint, regular, require
from mtk_readonly import ConnectedMtkReader, REVIEWED_REVISION
from mtk_session import Candidate, ReadPolicy, loader_bytes, source_pin
from mtk_usb import ExactUsbBackend, supervised_operations
from mtk_writer import ConnectedMtkWriter

MAX = 1024 * 1024
READABLE = IDENTITY_PARTITIONS | {'boot', 'recovery', 'odmdtbo'}


class Wire:
    def __init__(self, incoming, outgoing):
        self.incoming, self.outgoing = incoming, outgoing

    def exact(self, size):
        result = bytearray()
        while len(result) < size:
            block = self.incoming.read(size-len(result))
            require(block, 'Native host disconnected')
            result.extend(block)
        return bytes(result)

    def receive(self):
        size, = struct.unpack('<I', self.exact(4))
        require(0 < size <= MAX, 'RPC frame exceeds bound')
        value = json.loads(self.exact(size))
        require(isinstance(value, dict), 'Expected RPC object')
        return value

    def send(self, value):
        data = json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
        require(0 < len(data) <= MAX, 'RPC response exceeds bound')
        self.outgoing.write(struct.pack('<I', len(data)))
        self.outgoing.write(data)
        self.outgoing.flush()

    def chunk(self, data):
        require(0 < len(data) <= MAX, 'RPC data exceeds bound')
        self.outgoing.write(struct.pack('<III', len(data), len(data), 0))
        self.outgoing.write(data)
        self.outgoing.flush()

    @contextmanager
    def deadline(self, seconds):
        require(isinstance(seconds, (int, float)) and 0 < seconds <= 900, 'Invalid operation deadline')
        self.send({'event': 'deadline', 'seconds': seconds})
        require(self.receive() == {'ack': 'deadline'}, 'Native supervisor did not arm deadline')
        yield


class Adapter:
    def __init__(self, wire, *, backend=ExactUsbBackend, reader=ConnectedMtkReader, writer=ConnectedMtkWriter):
        self.wire = wire
        self.backend_factory, self.reader_factory, self.writer_factory = backend, reader, writer
        self.backend = self.reader = self.writer = None
        self.loader = self.stage = self.binding = None
        self.written = False
        self.verified = False

    def prepare(self, command):
        require(self.backend is None, 'Adapter already prepared')
        require(set(command) == {'op', 'checkout', 'loader', 'loader_sha256', 'preloader', 'preloader_sha256', 'libusb'}, 'Unexpected preparation fields')
        source_pin(command['checkout'])
        self.loader = loader_bytes(command['loader'], command['loader_sha256'])
        self.backend = self.backend_factory(Path(command['checkout']), preloader=command['preloader'],
            preloader_sha256=command['preloader_sha256'], libusb_path=command['libusb'])
        self.backend.prepare(self.loader)
        self.wire.send({'event': 'prepared'})

    def start(self, command):
        require(self.backend is not None and self.reader is None, 'Expected prepared fresh adapter')
        require(set(command) == {'op', 'candidate'}, 'Unexpected startup fields')
        raw = command['candidate']
        require(isinstance(raw, dict) and set(raw) == {'bus', 'address', 'ports', 'vid', 'pid'}, 'Invalid USB selection')
        require(all(type(raw[n]) is int and raw[n] > 0 for n in ('bus', 'address', 'vid', 'pid'))
                and isinstance(raw['ports'], list) and raw['ports']
                and all(type(n) is int and 0 < n <= 255 for n in raw['ports']), 'Invalid USB topology')
        selected = Candidate(raw['bus'], raw['address'], tuple(raw['ports']), raw['vid'], raw['pid'])
        self.backend.claim(selected)
        require(self.backend.claimed_candidate() == selected, 'Claimed USB selection differs')
        mtk = self.backend.start_readonly(self.loader, ReadPolicy())
        self.reader = self.reader_factory(mtk, REVIEWED_REVISION)
        self.wire.send({'event': 'connected', 'device': self.reader.description})

    def authorize_boot(self, command):
        require(self.reader is not None and self.writer is None and not self.written, 'Expected fresh read-only session')
        require(set(command) == {'op', 'cid_sha256', 'partitions', 'identity_sha256', 'original_sha256', 'stage', 'stage_sha256'}, 'Unexpected boot admission fields')
        desc = self.reader.description
        require(desc['runtime_cid_sha256'] == command['cid_sha256'] and desc['partitions'] == command['partitions'], 'Observed device differs from admitted baseline')
        hashes = command['identity_sha256']
        originals = command['original_sha256']
        require(isinstance(hashes, dict) and set(hashes) == IDENTITY_PARTITIONS
                and isinstance(originals, dict) and set(originals) == {'boot', 'recovery', 'odmdtbo'}, 'Incomplete original identity evidence')
        for name, checksum in {**hashes, **originals}.items():
            require(self.reader.hash(name) == checksum, 'Device changed after original capture')
        stage = regular(command['stage']).resolve()
        require(stage.stat().st_size == desc['partitions']['boot']['size'] == 16*1024*1024, 'RAM stage must fill one boot partition')
        with stage.open('rb') as file:
            require(file.read(8) == b'ANDROID!', 'Invalid RAM boot image')
        binding = {**desc, 'model': MODEL, 'model_verified': True, 'identity_sha256': fingerprint(hashes)}
        release = {'schema': 1, 'model': MODEL, 'partitions': desc['partitions'],
                   'images': {'boot': {'file': stage.name, 'sha256': command['stage_sha256']}}}
        self.writer = self.writer_factory(self.backend.mtk, REVIEWED_REVISION, release=release,
            bundle=stage.parent, binding=binding, progress=lambda phase, target, done, total:
                self.wire.send({'event': 'progress', 'phase': phase, 'target': target, 'done': done, 'total': total}))
        self.reader = self.writer
        self.stage = stage
        self.binding = command
        self.wire.send({'event': 'boot_authorized'})

    def dispatch(self, command):
        op = command.get('op')
        if op == 'prepare':
            return self.prepare(command)
        if op == 'enumerate':
            require(self.backend is not None and self.reader is None and set(command) == {'op'}, 'Unexpected enumeration')
            with self.wire.deadline(10):
                found = self.backend.enumerate()
            self.wire.send({'event': 'candidates', 'devices': [dict(bus=d.bus, address=d.address, ports=list(d.ports), vid=d.vid, pid=d.pid) for d in found]})
        elif op == 'start':
            self.start(command)
        elif op in ('read', 'hash'):
            require(self.reader is not None and set(command) == {'op', 'target'} and command['target'] in READABLE, 'Unsupported read target')
            name = command['target']
            if op == 'hash':
                self.wire.send({'event': 'hash', 'target': name, 'sha256': self.reader.hash(name)})
            else:
                self.wire.send({'event': 'partition', 'target': name, 'size': self.reader.description['partitions'][name]['size']})
                full = hashlib.sha256()
                for data in self.reader.chunks(name):
                    full.update(data)
                    self.wire.chunk(data)
                self.wire.send({'event': 'read_complete', 'target': name, 'sha256': full.hexdigest()})
        elif op == 'authorize_boot':
            self.authorize_boot(command)
        elif op == 'write_boot':
            require(set(command) == {'op'} and self.writer is not None and not self.written, 'Boot write not admitted or already consumed')
            self.written = True  # Even an ambiguous first packet consumes this attempt.
            self.writer.write('boot', self.stage)
            require(self.writer.hash('boot') == self.binding['stage_sha256'], 'RAM boot readback differs')
            for name, checksum in {**self.binding['identity_sha256'], **{k:v for k,v in self.binding['original_sha256'].items() if k != 'boot'}}.items():
                require(self.writer.hash(name) == checksum, 'Retained originals changed after bootstrap')
            self.verified = True
            self.wire.send({'event': 'boot_verified'})
        elif op == 'boot':
            require(set(command) == {'op'} and self.verified and self.writer is not None, 'No verified boot operation')
            self.backend.boot_after_capture()
            self.wire.send({'event': 'boot_requested'})
        elif op == 'close':
            require(set(command) == {'op'}, 'Unexpected close fields')
            self.close()
            self.wire.send({'event': 'closed'})
            return False
        else:
            require(False, 'Unsupported native adapter operation')
        return True

    def close(self):
        if self.writer is not None:
            self.writer.close()
            self.writer = None
        if self.backend is not None:
            self.backend.close(reset=False)
            self.backend = None


def serve(wire, factory=Adapter):
    adapter = factory(wire)
    try:
        with supervised_operations(wire.deadline):
            while adapter.dispatch(wire.receive()) is not False:
                pass
    finally:
        adapter.close()


if __name__ == '__main__':
    require(sys.argv[1:] == ['--events-stdio'], 'Use the native host event channel')
    incoming, outgoing = sys.stdin.buffer, sys.stdout.buffer
    # Upstream diagnostics must never interleave with the binary RPC channel.
    with open(__import__('os').devnull, 'w') as quiet:
        sys.stdout = sys.stderr = quiet
        wire = Wire(incoming, outgoing)
        try:
            serve(wire)
        except BaseException:
            try:
                wire.send({'event': 'error', 'preserve_originals': True})
            except BaseException:
                pass
            raise SystemExit(1)
