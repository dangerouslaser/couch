from contextlib import contextmanager, redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

from benchmark_stream import benchmark
from couch_install import IDENTITY_PARTITIONS, InstallError
from mtk_readonly import ConnectedMtkReader, REVIEWED_REVISION
from test_mtk_readonly import fake_session


class BenchmarkTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        data = root / "input.bin"
        data.write_bytes(b"fixture")
        sha = hashlib.sha256(b"fixture").hexdigest()
        self.mtk, _, _ = fake_session()
        self.legacy = ConnectedMtkReader(self.mtk, REVIEWED_REVISION)
        self.baseline = {"schema": 1, "capacity": self.legacy.capacity,
                         "partitions": self.legacy.description["partitions"],
                         "cid": "000000007b00000000000000c8010000",
                         "identity_sha256": {name: self.legacy.hash(name) for name in IDENTITY_PARTITIONS}}
        baseline = root / "baseline.json"
        baseline.write_text(json.dumps(self.baseline))
        self.args = NS(bus=1, ports="1", timeout=1, baseline=baseline,
                       checkout=root, loader=data, loader_sha256=sha, preloader=data,
                       preloader_sha256=sha, check_only=False, lock_dir=root / "lock",
                       report=root / "result.json", boot_after_capture=True)
        self.events = []

    def session(self, *args, **kwargs):
        @contextmanager
        def context():
            self.events.append("session")
            try:
                yield kwargs["device_factory"](self.mtk, REVIEWED_REVISION)
                if kwargs["boot_after_capture"]:
                    self.events.append("exit")
            finally:
                self.events.append("close")
        return context()

    def legacy_factory(self, *args):
        reader = self.legacy
        original = reader.hash
        def record(name):
            self.events.append("legacy:" + name)
            return original(name)
        reader.hash = record
        return reader

    def stream_factory(self, *args):
        self.events.append("stream-created")
        expected = self.baseline["identity_sha256"]
        def read(name):
            self.events.append("stream:" + name)
            return expected[name]
        return NS(description={**self.legacy.description, "transport": "fixture-stream"}, hash=read, stats={})

    def run_benchmark(self, **kwargs):
        with patch("benchmark_stream.source_pin", return_value={}), redirect_stdout(io.StringIO()):
            return benchmark(self.args, session=self.session, legacy_factory=self.legacy_factory,
                             stream_factory=kwargs.get("stream_factory", self.stream_factory))

    def test_check_only_never_creates_or_waits_for_session(self):
        self.args.check_only = True
        def forbidden(*args, **kwargs):
            self.fail("offline check opened USB")
        with patch("benchmark_stream.source_pin", return_value={}), redirect_stdout(io.StringIO()):
            benchmark(self.args, session=forbidden, enumerate_devices=forbidden)
        self.assertFalse(self.args.report.exists())

    def test_full_baseline_then_probe_then_timings_then_successful_exit(self):
        result = self.run_benchmark()
        boundary = self.events.index("stream-created")
        self.assertEqual(set(self.events[1:boundary]), {"legacy:" + name for name in IDENTITY_PARTITIONS})
        self.assertEqual(self.events[boundary:], ["stream-created", "stream:proinfo", "legacy:nvdata", "stream:nvdata", "exit", "close"])
        self.assertTrue(result["complete"] and result["checksum_probe_verified"] and result["boot_acknowledged"])
        self.assertFalse(result["writes_sent"] or result["normal_os_verified"])
        saved = self.args.report.read_text()
        self.assertNotIn(self.baseline["cid"], saved)
        self.assertNotIn(self.legacy.description["storage_id"], saved)

    def test_baseline_mismatch_prevents_stream_creation(self):
        self.baseline["identity_sha256"]["nvram"] = "0" * 64
        self.args.baseline.write_text(json.dumps(self.baseline))
        with self.assertRaisesRegex(InstallError, "baseline differs"):
            self.run_benchmark()
        self.assertNotIn("stream-created", self.events)
        self.assertNotIn("exit", self.events)
        self.assertEqual(self.events[-1], "close")

    def test_probe_failure_prevents_large_read_and_exit(self):
        def factory(*args):
            reader = self.stream_factory(*args)
            def failed(name):
                self.events.append("stream:" + name)
                raise InstallError("Streaming read checksum mismatch")
            reader.hash = failed
            return reader
        with self.assertRaisesRegex(InstallError, "checksum"):
            self.run_benchmark(stream_factory=factory)
        self.assertNotIn("stream:nvdata", self.events)
        self.assertNotIn("exit", self.events)
        self.assertFalse(self.args.report.exists())

    def test_complete_baseline_and_bounded_targets_required_offline(self):
        for change in (lambda b: b.pop("cid"), lambda b: b["identity_sha256"].clear(),
                       lambda b: b["partitions"]["proinfo"].update(size=8 * 1024 * 1024)):
            value = json.loads(json.dumps(self.baseline))
            change(value)
            self.args.baseline.write_text(json.dumps(value))
            with self.assertRaises(InstallError):
                self.run_benchmark()
        self.assertEqual(self.events, [])


if __name__ == "__main__":
    unittest.main()
