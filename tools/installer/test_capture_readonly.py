from contextlib import contextmanager, redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

from couch_install import IDENTITY_PARTITIONS, InstallError, read_json
from capture_readonly import capture, compare_baseline, wait_preloader
from mtk_readonly import ConnectedMtkReader, REVIEWED_REVISION
from mtk_session import Candidate
from test_mtk_readonly import fake_session


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        data = self.root / "input.bin"
        data.write_bytes(b"fixture")
        sha = hashlib.sha256(b"fixture").hexdigest()
        self.args = NS(bus=1, ports="1", timeout=1, backup_dir=self.root / "capture", baseline=None,
                       checkout=self.root, loader=data, loader_sha256=sha, preloader=data,
                       preloader_sha256=sha, check_only=False, lock_dir=self.root / "lock")
        self.candidate = Candidate(1, 7, (1,), 0x0e8d, 0x2000)
        self.reader = ConnectedMtkReader(fake_session()[0], REVIEWED_REVISION)

    def test_check_only_never_enumerates_or_constructs_session(self):
        self.args.check_only = True
        def forbidden(*args):
            self.fail("check-only attempted USB")
        with patch("capture_readonly.source_pin", return_value={}), redirect_stdout(io.StringIO()):
            capture(self.args, enumerate_devices=forbidden, session=forbidden)
        self.assertFalse(self.args.backup_dir.exists())

    def test_complete_capture_matches_runtime_baseline_and_closes(self):
        baseline = {"schema": 1, "capacity": self.reader.capacity,
                    "partitions": self.reader.description["partitions"],
                    "cid": "000000007b00000000000000c8010000",
                    "identity_sha256": {name: self.reader.hash(name) for name in IDENTITY_PARTITIONS}}
        self.args.baseline = self.root / "baseline.json"
        self.args.baseline.write_text(json.dumps(baseline))
        closed = []
        @contextmanager
        def session(*args, **kwargs):
            try:
                yield self.reader
            finally:
                closed.append(True)
        with patch("capture_readonly.source_pin", return_value={}), redirect_stdout(io.StringIO()):
            capture(self.args, enumerate_devices=lambda: [self.candidate], session=session)
        report = read_json(self.args.backup_dir / "readback.json")
        self.assertTrue(report["complete"] and report["runtime_cid_verified"] and report["runtime_identity_verified"])
        self.assertEqual(closed, [True])

    def test_changed_cid_is_rejected_before_backing_up(self):
        baseline = {"capacity": self.reader.capacity, "partitions": self.reader.description["partitions"], "cid": "12" * 16}
        with self.assertRaisesRegex(InstallError, "CID differs"):
            compare_baseline(self.reader, baseline)

    def test_wire_cid_is_not_accepted_as_runtime_format(self):
        baseline = {"capacity": self.reader.capacity, "partitions": self.reader.description["partitions"],
                    "cid": struct.pack(">QQ", 123, 456).hex()}
        with self.assertRaisesRegex(InstallError, "CID differs"):
            compare_baseline(self.reader, baseline)

    def test_runtime_gadget_is_ignored_until_preloader_appears(self):
        observations = iter([[Candidate(1, 6, (1,), 0x0e8d, 0x201c)], [self.candidate]])
        found = wait_preloader(lambda: next(observations), 1, (1,), 1, sleep=lambda _: None)
        self.assertEqual(found, self.candidate)

    def test_wrong_port_multiple_devices_and_brom_are_rejected(self):
        for devices in ([Candidate(1, 7, (2,), 0x0e8d, 0x2000)],
                        [self.candidate, self.candidate], [Candidate(1, 7, (1,), 0x0e8d, 3)]):
            with self.subTest(devices=devices), self.assertRaises(InstallError):
                wait_preloader(lambda: devices, 1, (1,), 1)

    def test_wait_timeout_does_not_reset_or_reconnect(self):
        ticks = iter([0, 0, 2])
        with self.assertRaisesRegex(InstallError, "before timeout"):
            wait_preloader(lambda: [], 1, (1,), 1, clock=lambda: next(ticks), sleep=lambda _: None)


if __name__ == "__main__":
    unittest.main()
