from contextlib import contextmanager, redirect_stdout
import copy
import hashlib
import io
import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

from couch_install import IDENTITY_PARTITIONS, InstallError, MODEL, read_json
from mtk_readonly import ConnectedMtkReader, REVIEWED_REVISION
from stock_baseline_restore import TARGETS, load_stock_manifest, restore_stock, run
from test_mtk_readonly import fake_session


class StockRestoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bundle = self.root / "bundle"
        self.bundle.mkdir()
        self.mtk, _, _ = fake_session()
        self.reader = ConnectedMtkReader(self.mtk, REVIEWED_REVISION)
        partitions = copy.deepcopy(self.reader.description["partitions"])
        end = max(v["offset"] + v["size"] for v in partitions.values())
        partitions["odmdtbo"] = {"offset": end, "size": 512}
        self.reader.description["partitions"] = partitions
        self.release = dict(schema=1, model=MODEL, private_candidate=True, installable=False,
                            purpose="stock-baseline", partitions=partitions, images={})
        for name in TARGETS:
            data = bytearray(partitions[name]["size"])
            if name == "userdata":
                for pos in (1024, 5120):
                    struct.pack_into("<I", data, pos, 0xf2f52010)
            if name == "boot":
                data[:8] = b"ANDROID!"
            file = name + ".img"
            (self.bundle / file).write_bytes(data)
            self.release["images"][name] = dict(file=file, sha256=hashlib.sha256(data).hexdigest())
        self.manifest = self.bundle / "release.json"
        self.manifest.write_text(json.dumps(self.release))
        cid = "000000007b00000000000000c8010000"
        self.baseline = dict(schema=1, cid=cid, capacity=self.reader.capacity,
                             partitions=partitions, identity_sha256={name:self.reader.hash(name) for name in IDENTITY_PARTITIONS})
        path = self.root / "baseline.json"
        path.write_text(json.dumps(self.baseline))
        self.args = NS(allow_private_flash=True, discard_test_installation=True,
                       manifest=self.manifest, baseline=path, confirm_cid_sha256=hashlib.sha256(bytes.fromhex(cid)).hexdigest(),
                       checkout=self.root, loader=path, loader_sha256="a"*64, preloader=path,
                       preloader_sha256="b"*64, bus=1, ports="1", timeout=120,
                       journal_dir=self.root / "journal", lock_dir=self.root / "lock",
                       boot_after_restore=True, check_only=False)
        self.events = []
        self.fail = None

    def fake_writer(self, *args, **kwargs):
        self.events.append("writer-created")
        outer = self
        class Writer:
            description = {**outer.reader.description, "model": MODEL}
            def __enter__(self): return self
            def __exit__(self, *args): outer.events.append("writer-closed")
            def hash(self, name):
                outer.events.append("hash:" + name)
                if name in TARGETS:
                    return "0"*64 if outer.fail == "readback" else outer.release["images"][name]["sha256"]
                return outer.reader.hash(name)
            def write(self, name, source):
                record = read_json(outer.args.journal_dir / "journal.json")
                assert record["writes"][name] == "writing"
                outer.events.append("write:" + name)
                if outer.fail == name: raise InstallError("fixture write failure")
        return Writer()

    @contextmanager
    def session(self, *args, **kwargs):
        try:
            yield kwargs["device_factory"](self.mtk, REVIEWED_REVISION)
            if kwargs["boot_after_capture"]: self.events.append("exit")
        finally:
            self.events.append("session-closed")

    def invoke(self):
        with patch("stock_baseline_restore.source_pin"), patch("stock_baseline_restore.loader_bytes"), redirect_stdout(io.StringIO()):
            return run(self.args, session=self.session, reader_factory=lambda *a:self.reader, writer_factory=self.fake_writer)

    def test_stock_format_and_no_recovery_manifest_are_valid_without_public_changes(self):
        release, _ = load_stock_manifest(self.manifest)
        self.assertEqual(set(release["images"]), set(TARGETS))
        from couch_install import load_release
        with self.assertRaises(InstallError): load_release(self.manifest)

    def test_explicit_discard_and_private_flags_before_usb(self):
        for flag in ("discard_test_installation", "allow_private_flash"):
            setattr(self.args, flag, False)
            with self.assertRaises(InstallError): self.invoke()
            setattr(self.args, flag, True)
        self.assertEqual(self.events, [])

    def test_check_only_never_starts_session_or_journal(self):
        self.args.check_only = True
        self.invoke()
        self.assertEqual(self.events, [])
        self.assertFalse(self.args.journal_dir.exists())

    def test_exact_order_independent_readback_identity_and_retained_recovery(self):
        self.invoke()
        self.assertEqual([e for e in self.events if e.startswith("write:")], ["write:"+name for name in TARGETS])
        self.assertEqual(self.events.count("hash:recovery"), 2)
        for name in TARGETS:
            i = self.events.index("write:" + name)
            self.assertEqual(self.events[i+1], "hash:" + name)
        record = read_json(self.args.journal_dir / "journal.json")
        self.assertTrue(record["complete"] and record["identity_verified_after"] and record["recovery_verified_after"])
        self.assertFalse(record["userdata_backup_taken"] or record["normal_os_verified"])
        self.assertEqual(record["writes"], {name:"verified" for name in TARGETS})
        self.assertEqual(self.events[-3:], ["writer-closed", "exit", "session-closed"])

    def test_failed_write_stops_no_boot_no_retry_and_retains_journal(self):
        self.fail = "odmdtbo"
        with self.assertRaisesRegex(InstallError, "write failure"): self.invoke()
        record = read_json(self.args.journal_dir / "journal.json")
        self.assertEqual(record["writes"], {"userdata":"verified", "odmdtbo":"writing"})
        self.assertFalse(record["complete"])
        self.assertNotIn("write:boot", self.events)
        self.assertNotIn("exit", self.events)
        before = list(self.events)
        with self.assertRaisesRegex(InstallError, "new stock-restore journal"): self.invoke()
        self.assertEqual(self.events, before)

    def test_failed_readback_never_writes_next_target(self):
        self.fail = "readback"
        with self.assertRaisesRegex(InstallError, "independent readback"): self.invoke()
        self.assertEqual([e for e in self.events if e.startswith("write:")], ["write:userdata"])
        self.assertNotIn("exit", self.events)

    def test_calibration_mismatch_prevents_writer_creation(self):
        self.baseline["identity_sha256"]["nvram"] = "0"*64
        self.args.baseline.write_text(json.dumps(self.baseline))
        with self.assertRaisesRegex(InstallError, "baseline differs"): self.invoke()
        self.assertNotIn("writer-created", self.events)

    def test_late_image_corruption_and_extra_target_fail_before_usb(self):
        (self.bundle / "boot.img").write_bytes(b"invalid")
        with self.assertRaises(InstallError): self.invoke()
        self.assertEqual(self.events, [])
        self.release["images"]["recovery"] = self.release["images"]["boot"]
        self.manifest.write_text(json.dumps(self.release))
        with self.assertRaisesRegex(InstallError, "exactly"): self.invoke()
        self.assertEqual(self.events, [])

    def test_both_f2fs_superblocks_required_even_with_matching_hash(self):
        path = self.bundle / "userdata.img"
        data = bytearray(path.read_bytes())
        data[5120:5124] = b"bad!"
        path.write_bytes(data)
        self.release["images"]["userdata"]["sha256"] = hashlib.sha256(data).hexdigest()
        self.manifest.write_text(json.dumps(self.release))
        with self.assertRaisesRegex(InstallError, "both F2FS"): self.invoke()
        self.assertEqual(self.events, [])


if __name__ == "__main__": unittest.main()
