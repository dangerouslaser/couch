"""One supervised, no-capture debug boot transition; separate from attachment."""
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from couch_install import require
from mtk_adapter import serve_stdio
from mtk_session import ReadPolicy, loader_bytes, source_pin, verify_loaded_sources
from mtk_usb import ExactUsbBackend
from mtk_readonly import ConnectedMtkReader, REVIEWED_REVISION
from mtk_writer import ConnectedMtkWriter
import wifi_debug_transition as transition
import recover_native_bootstrap as recovery

DA_SHA256 = 'aef234190ccb8145d2e3b8459741e9adb70f2caa8481aa216c1b25152afaca1f'
PRELOADER_SHA256 = '0ad0d14b7203d98a6567af7a022cfe5df5b6fcbba60cb4e9b4bc2ee569cf1069'


class TransitionWorker:
    def __init__(self, wire):
        self.wire = wire
        self.proof = None
        self.backend = None
        self.verified = False
        self.consumed = False
        self.boot_requested = False

    def dispatch(self, command):
        require(isinstance(command, dict) and set(command) == {'op', 'payload'}, 'Invalid transition request')
        op, payload = command['op'], command['payload']
        if op == 'transition_check':
            require(payload is None and self.proof is None and not self.consumed, 'Invalid offline check')
            self.wire.send({'event': op, 'result': {'device_access': False, 'ready': True}})
        elif op == 'transition_validate_receipt':
            require(self.proof is None and self.backend is None and not self.consumed,
                    'Receipt validation must be offline')
            require(isinstance(payload, dict) and set(payload) == {'config', 'bus', 'ports'},
                    'Invalid receipt validation fields')
            self.wire.send({'event': op, 'result': transition.validate_receipt(**payload)})
        elif op == 'transition_admit':
            require(self.proof is None and not self.consumed and isinstance(payload, dict), 'Transition already admitted')
            require(set(payload) == {'source', 'temporary_boot_sha256', 'original_boot_sha256',
                'snapshot_sha256', 'image', 'image_sha256', 'metadata', 'metadata_sha256'}, 'Invalid admission fields')
            self.proof = transition.admit(**payload)
            self.wire.send({'event': op, 'result': {'admitted': True, 'device_access': False}})
        elif op == 'transition_execute':
            require(self.proof is not None and not self.consumed, 'Transition unavailable or consumed')
            self.consumed = True
            require(isinstance(payload, dict) and set(payload) == {'bus', 'ports', 'runtime', 'libusb', 'output'},
                    'Invalid transition execution fields')
            require(type(payload['bus']) is int and 0 < payload['bus'] <= 255
                    and isinstance(payload['ports'], list) and 1 <= len(payload['ports']) <= 7
                    and all(type(p) is int and 0 < p <= 255 for p in payload['ports']), 'Invalid exact USB port')
            # Recheck retained admission before even enumerating USB.
            checked = transition.admit(self.proof.retained.source,
                temporary_boot_sha256=self.proof.retained.temporary_boot_sha256,
                original_boot_sha256=self.proof.original_boot_sha256, snapshot_sha256=self.proof.snapshot_sha256,
                image=self.proof.image, image_sha256=self.proof.image_sha256,
                metadata=self.proof.metadata, metadata_sha256=self.proof.metadata_sha256)
            require(checked == self.proof, 'Transition inputs changed')
            root = self.proof.retained.source
            runtime = Path(payload['runtime'])
            pinned = source_pin(runtime/'mtk')
            loader = loader_bytes(root/'dependencies/owner-da/loader.bin', DA_SHA256)
            self.backend = ExactUsbBackend(runtime/'mtk',
                preloader=root/'owner-inputs/bootstrap/preloader.img', preloader_sha256=PRELOADER_SHA256,
                libusb_path=payload['libusb'])
            self.backend.prepare(loader)
            verify_loaded_sources(pinned)
            deadline = time.monotonic() + 180
            selected = None
            while time.monotonic() < deadline:
                matches = [d for d in self.backend.enumerate() if d.bus == payload['bus']
                           and d.ports == tuple(payload['ports']) and d.vid == 0x0e8d and d.pid == 0x2000]
                require(len(matches) <= 1, 'Multiple selected preloaders')
                if matches:
                    selected = matches[0]
                    break
                time.sleep(0.05)
            require(selected is not None, 'Selected preloader did not appear; no write attempted')
            self.backend.claim(selected)
            require(self.backend.claimed_candidate() == selected, 'Claimed port changed')
            mtk = self.backend.start_readonly(loader, ReadPolicy())
            verify_loaded_sources(pinned)
            reader = ConnectedMtkReader(mtk, REVIEWED_REVISION)
            def factory(release, bundle, binding):
                require(self.backend.claimed_candidate() == selected and self.backend.mtk is mtk,
                        'Transition connection changed')
                return ConnectedMtkWriter(mtk, REVIEWED_REVISION, release=release, bundle=bundle, binding=binding,
                    progress=lambda phase, target, done, total: self.wire.send({'event': 'progress',
                        'phase': phase, 'target': target, 'done': done, 'total': total}))
            result = transition.transition(self.proof, reader, factory, payload['output'],
                                           bus=payload['bus'], ports=payload['ports'])
            self.output = Path(payload['output'])
            self.verified = True
            self.wire.send({'event': op, 'result': result})
        elif op == 'transition_boot':
            require(payload is None and self.verified and self.backend is not None and not self.boot_requested,
                    'Only a verified transition may request one boot')
            self.boot_requested = True
            recovery.publish(self.output, 'restart-requested.json', {'requested': True, 'acknowledged': False})
            self.backend.boot_after_capture()
            recovery.publish(self.output, 'restart-acknowledged.json', {'requested': True, 'acknowledged': True})
            pin = transition.complete_receipt(self.output)
            self.wire.send({'event': op, 'result': {'acknowledged': True, 'receipt_sha256': pin}})
            return False
        elif op == 'transition_close':
            require(payload is None, 'Unexpected close payload')
            self.wire.send({'event': op, 'result': {'closed': True}})
            return False
        else:
            require(False, 'Unsupported transition operation')

    def close(self):
        if self.backend is not None:
            self.backend.close(reset=False)
            self.backend = None


if __name__ == '__main__':
    require(sys.argv[1:] == ['--events-stdio'], 'Use the native supervisor')
    raise SystemExit(serve_stdio(TransitionWorker))
