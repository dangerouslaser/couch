import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

from couch_install import InstallError
from wifi_debug_transition_worker import TransitionWorker


class TransitionWorkerTests(unittest.TestCase):
    def worker(self):
        return TransitionWorker(SimpleNamespace(send=Mock()))

    def test_offline_check_and_close_never_construct_usb_backend(self):
        with patch('wifi_debug_transition_worker.ExactUsbBackend') as backend:
            worker = self.worker()
            worker.dispatch({'op': 'transition_check', 'payload': None})
            worker.wire.send.assert_called_once_with({'event': 'transition_check',
                'result': {'device_access': False, 'ready': True}})
            self.assertFalse(worker.dispatch({'op': 'transition_close', 'payload': None}))
            backend.assert_not_called()

    def test_no_boot_or_execute_without_admission_and_verification(self):
        for op in ('transition_boot', 'transition_execute', 'stage_provision', 'write', 'boot', 'exec'):
            worker = self.worker()
            worker.backend = Mock()
            with self.assertRaises(InstallError):
                worker.dispatch({'op': op, 'payload': None})
            worker.backend.assert_not_called()
            self.assertEqual(worker.backend.method_calls, [])
            worker.wire.send.assert_not_called()

    def test_missing_or_tampered_receipt_never_constructs_backend(self):
        with patch('wifi_debug_transition_worker.ExactUsbBackend') as backend:
            worker = self.worker()
            with self.assertRaises(InstallError):
                worker.dispatch({'op':'transition_validate_receipt',
                    'payload':{'config':{}, 'bus':1, 'ports':[2]}})
            backend.assert_not_called()
            worker.wire.send.assert_not_called()

    def test_changed_evidence_fails_before_backend_and_consumes_attempt(self):
        worker = self.worker()
        worker.proof = SimpleNamespace(retained=SimpleNamespace(source='/retained',
            temporary_boot_sha256='a'*64), original_boot_sha256='b'*64,
            snapshot_sha256='c'*64, image='/debug.img', image_sha256='d'*64,
            metadata='/debug.json', metadata_sha256='e'*64)
        request = {'op': 'transition_execute', 'payload': {'bus': 1, 'ports': [2],
            'runtime': '/runtime', 'libusb': '/libusb', 'output': '/output'}}
        with patch('wifi_debug_transition_worker.transition.admit', return_value=object()), \
                patch('wifi_debug_transition_worker.ExactUsbBackend') as backend:
            for _ in range(2):
                with self.assertRaises(InstallError): worker.dispatch(request)
            backend.assert_not_called()
        self.assertTrue(worker.consumed)

    def test_ambiguous_boot_is_not_retried_or_acknowledged(self):
        worker = self.worker()
        worker.backend = Mock()
        worker.backend.boot_after_capture.side_effect = RuntimeError('ambiguous')
        worker.verified, worker.output = True, '/new-receipt'
        with patch('wifi_debug_transition_worker.recovery.publish') as publish:
            with self.assertRaises(RuntimeError):
                worker.dispatch({'op': 'transition_boot', 'payload': None})
            with self.assertRaises(InstallError):
                worker.dispatch({'op': 'transition_boot', 'payload': None})
            publish.assert_called_once_with('/new-receipt', 'restart-requested.json',
                {'requested': True, 'acknowledged': False})
        worker.backend.boot_after_capture.assert_called_once_with()
        worker.wire.send.assert_not_called()


if __name__ == '__main__': unittest.main()
