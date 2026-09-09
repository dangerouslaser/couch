import hashlib
import importlib
from pathlib import Path
import signal
import sys
import tempfile
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

from couch_install import InstallError
from mtk_session import ReadPolicy, read_session
from mtk_usb import ExactUsbBackend, PinnedImports, bounded_operation, descriptor
from test_mtk_readonly import fake_session


class Interface(list):
    bAlternateSetting = 0
    bInterfaceClass = 10
    bInterfaceNumber = 1


class Configuration(list):
    bNumInterfaces = 2


class UsbBackendTests(unittest.TestCase):
    def setUp(self):
        self.events = []
        interface = Interface([NS(bmAttributes=2, bEndpointAddress=0x81), NS(bmAttributes=2, bEndpointAddress=2)])
        self.dev = NS(bus=1, address=2, port_numbers=(3,), idVendor=0x0e8d, idProduct=0x2000,
                      get_active_configuration=lambda: Configuration([interface]),
                      is_kernel_driver_active=lambda number: True,
                      detach_kernel_driver=lambda number: self.events.append(("detach", number)),
                      attach_kernel_driver=lambda number: self.events.append(("attach", number)))
        self.usb = NS(core=NS(find=lambda **kwargs: [self.dev]),
                      util=NS(claim_interface=lambda dev, number: self.events.append(("claim", number)),
                              release_interface=lambda dev, number: self.events.append(("release", number)),
                              dispose_resources=lambda dev: self.events.append("dispose")))
        self.security = 0
        self.hwcode = 0x6580
        self.upload_result = True
        self.handshake_result = True
        self.backend = ExactUsbBackend("unused-test-checkout", usb=self.usb, bindings=(self.config, self.mtk))
        self.addCleanup(self.backend.close)

    def config(self, **kwargs):
        config = NS(iot=False, is_brom=False)
        config.init_hwcode = lambda code: setattr(config, "hwcode", code)
        return config

    def mtk(self, config, **kwargs):
        mtk, _, _ = fake_session()
        mtk.config = config
        mtk.port = NS(cdc=NS(), run_handshake=lambda retries: self.handshake_result)
        def initialize(**kwargs):
            mtk.port.handshake()
            config.init_hwcode(self.hwcode)
            config.target_config = mtk.preloader.get_target_config()
            return True
        mtk.preloader = NS(init=initialize,
                           echo=lambda value: True,
                           Cmd=NS(GET_TARGET_CONFIG=NS(value=b"x")),
                           rbyte=lambda length: self.security.to_bytes(4, "big") + b"\0\0")
        def upload(**kwargs):
            self.events.append("upload")
            self.assertTrue(config.stock and config.skipwdt)
            self.assertFalse(config.reconnect)
            self.assertEqual(Path(config.loader).read_bytes(), b"loader")
            return self.upload_result
        mtk.daloader.upload_da = upload
        return mtk

    def start(self):
        self.backend.claim(descriptor(self.dev))
        return self.backend.start_readonly(b"loader", ReadPolicy())

    def test_claims_exact_descriptor_and_restores_interfaces(self):
        self.backend.claim(descriptor(self.dev))
        self.assertIs(self.backend.device, self.dev)
        self.assertEqual(self.dev.default_timeout, 1000)
        self.backend.close()
        self.assertEqual(self.events, [("detach", 0), ("claim", 0), ("detach", 1), ("claim", 1),
                                       ("release", 1), ("release", 0), ("attach", 1), ("attach", 0), "dispose"])

    def test_partial_claim_failure_is_cleaned_up(self):
        def claim(dev, number):
            if number == 1:
                raise OSError("claim failed")
        self.usb.util.claim_interface = claim
        with self.assertRaises(OSError):
            self.backend.claim(descriptor(self.dev))
        self.backend.close()
        self.assertIn(("release", 0), self.events)
        self.assertIn(("attach", 1), self.events)
        self.assertIn("dispose", self.events)

    def test_direct_da_start_disables_reconnect_and_write_entries(self):
        mtk = self.start()
        self.assertIn("upload", self.events)
        for function in (mtk.port.cdc.connect, mtk.port.close, mtk.bypass_security,
                         mtk.daloader.writeflash, mtk.daloader.formatflash):
            with self.assertRaises(InstallError):
                function()
        work = self.backend.work.name
        self.backend.close()
        self.assertFalse(Path(work).exists())

    def test_session_gate_runs_concrete_backend_with_mock_usb_end_to_end(self):
        with tempfile.TemporaryDirectory() as root:
            loader = Path(root) / "loader.bin"
            loader.write_bytes(b"loader")
            checksum = hashlib.sha256(b"loader").hexdigest()
            with patch("mtk_session.source_pin", return_value={}):
                with read_session(root, loader, checksum, Path(root) / "locks", descriptor(self.dev),
                                  lambda checkout: self.backend) as reader:
                    report = reader.backup_identity(Path(root) / "originals", reader.description["storage_id"])
                    self.assertTrue(report["complete"])
            self.assertIn("dispose", self.events)

    def test_wrong_chip_and_protected_targets_never_upload(self):
        for code, security in ((0x1234, 0), (0x6580, 1)):
            with self.subTest(code=code, security=security):
                self.hwcode, self.security = code, security
                with self.assertRaises(InstallError):
                    self.start()
                self.assertNotIn("upload", self.events)
                self.backend.close()
                self.backend.started = False

    def test_failed_handshake_does_not_retry_or_upload(self):
        self.handshake_result = False
        with self.assertRaisesRegex(InstallError, "handshake failed"):
            self.start()
        self.assertNotIn("upload", self.events)

    def test_missing_security_response_is_not_treated_as_unprotected(self):
        original = self.mtk
        def mtk(config, **kwargs):
            result = original(config, **kwargs)
            result.preloader.echo = lambda value: False
            return result
        self.backend.bindings = (self.config, mtk)
        with self.assertRaisesRegex(InstallError, "query unsupported"):
            self.start()
        self.assertNotIn("upload", self.events)

    def test_deadline_escapes_and_restores_timer(self):
        previous = signal.getsignal(signal.SIGALRM)
        with self.assertRaisesRegex(InstallError, "deadline exceeded"):
            with bounded_operation(0.01):
                time.sleep(1)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))
        self.assertEqual(signal.getsignal(signal.SIGALRM), previous)

    def test_pinned_importer_compiles_source_without_using_python_cache(self):
        with tempfile.TemporaryDirectory() as root:
            package = Path(root) / "mtkclient"
            package.mkdir()
            source = package / "__init__.py"
            source.write_text("VALUE = 'verified source'\n")
            (package / "__init__.pyc").write_bytes(b"not valid bytecode")
            pinned = {source.resolve(): hashlib.sha256(source.read_bytes()).hexdigest()}
            with patch("mtk_usb.source_pin", return_value=pinned):
                finder = PinnedImports(root)
                sys.meta_path.insert(0, finder)
                try:
                    self.assertEqual(importlib.import_module("mtkclient").VALUE, "verified source")
                finally:
                    finder.close()
            self.assertNotIn("mtkclient", sys.modules)


if __name__ == "__main__":
    unittest.main()
