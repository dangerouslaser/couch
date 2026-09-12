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

from couch_install import (FACTORY_WRITE_ORDER, IDENTITY_PARTITIONS, InstallError, MODEL,
                           NEVER_WRITE, WRITE_ORDER, allowed_write_targets, read_json)
from mtk_readonly import ConnectedMtkReader, REVIEWED_REVISION
from factory_restore import load_factory_manifest, run
from test_mtk_readonly import fake_session

EXT4 = bytes(0x438) + b"\x53\xef"
F2FS = {1024: struct.pack("<I", 0xF2F52010), 5120: struct.pack("<I", 0xF2F52010)}


class AllowedTargetTests(unittest.TestCase):
    def test_factory_purpose_unlocks_the_broad_set_only(self):
        self.assertEqual(allowed_write_targets({"purpose": "factory-restore"}), set(FACTORY_WRITE_ORDER))
        self.assertEqual(allowed_write_targets({"purpose": "stock-baseline"}), set(WRITE_ORDER))
        self.assertEqual(allowed_write_targets({}), set(WRITE_ORDER))

    def test_neither_order_can_name_a_protected_partition(self):
        self.assertTrue(set(WRITE_ORDER).isdisjoint(NEVER_WRITE))
        self.assertTrue(set(FACTORY_WRITE_ORDER).isdisjoint(NEVER_WRITE))
        for name in ("preloader", *IDENTITY_PARTITIONS):
            self.assertIn(name, NEVER_WRITE)
            self.assertNotIn(name, FACTORY_WRITE_ORDER)


class FactoryRestoreTests(unittest.TestCase):
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
        # Ensure every factory target exists in the layout with a sane size.
        sizes = {"system": 0x2000, "vendor": 0x1000, "secro": 0x800, "odmdtbo": 0x800,
                 "logo": 0x800, "cache": 0x1000, "userdata": 0x2000, "recovery": 0x800,
                 "lk2": 0x200, "lk": 0x200, "boot": 0x800}
        for name, size in sizes.items():
            if name not in partitions:
                partitions[name] = {"offset": end, "size": size}
                end += size
        self.reader.description["partitions"] = partitions
        self.partitions = partitions
        self.release = dict(schema=1, model=MODEL, private_candidate=True, installable=False,
                            purpose="factory-restore", source={"version": "V1.0.4"},
                            partitions=partitions, images={})
        for name in FACTORY_WRITE_ORDER:
            data = bytearray(partitions[name]["size"])
            if name == "boot":
                data[:8] = b"ANDROID!"
            if name in ("system", "vendor"):
                data[0x438:0x43a] = b"\x53\xef"
            if name == "userdata":
                for pos, magic in F2FS.items():
                    data[pos:pos + 4] = magic
            file = name + ".img"
            (self.bundle / file).write_bytes(data)
            self.release["images"][name] = dict(file=file, sha256=hashlib.sha256(data).hexdigest())
        self.manifest = self.bundle / "release.json"
        self.manifest.write_text(json.dumps(self.release))
        cid = "000000007b00000000000000c8010000"
        self.baseline = dict(schema=1, cid=cid, capacity=self.reader.capacity, partitions=partitions,
                             identity_sha256={name: self.reader.hash(name) for name in IDENTITY_PARTITIONS})
        self.baseline_path = self.root / "baseline.json"
        self.baseline_path.write_text(json.dumps(self.baseline))
        self.args = NS(allow_private_flash=True, confirm_factory_reset=True, manifest=self.manifest,
                       baseline=self.baseline_path,
                       confirm_cid_sha256=hashlib.sha256(bytes.fromhex(cid)).hexdigest(),
                       checkout=self.root, loader=self.baseline_path, loader_sha256="a" * 64,
                       preloader=self.baseline_path, preloader_sha256="b" * 64, bus=1, ports="1",
                       timeout=120, journal_dir=self.root / "journal", lock_dir=self.root / "lock",
                       boot_after_restore=True, check_only=False)
        self.events = []
        self.fail = None

    def fake_writer(self, *args, **kwargs):
        self.events.append("writer-created")
        outer = self

        class Writer:
            description = {**outer.reader.description, "model": MODEL}

            def __enter__(self): return self

            def __exit__(self, *a): outer.events.append("writer-closed")

            def hash(self, name):
                outer.events.append("hash:" + name)
                if name in outer.release["images"]:
                    return "0" * 64 if outer.fail == "readback" else outer.release["images"][name]["sha256"]
                return outer.reader.hash(name)

            def write(self, name, source):
                record = read_json(outer.args.journal_dir / "journal.json")
                assert record["writes"][name] == "writing"
                outer.events.append("write:" + name)
                if outer.fail == name:
                    raise InstallError("fixture write failure")
        return Writer()

    @contextmanager
    def session(self, *args, **kwargs):
        try:
            yield kwargs["device_factory"](self.mtk, REVIEWED_REVISION)
            if kwargs["boot_after_capture"]:
                self.events.append("exit")
        finally:
            self.events.append("session-closed")

    def invoke(self):
        with patch("factory_restore.source_pin"), patch("factory_restore.loader_bytes"), \
                redirect_stdout(io.StringIO()):
            return run(self.args, session=self.session, reader_factory=lambda *a: self.reader,
                       writer_factory=self.fake_writer)

    def test_manifest_loads_with_full_factory_set(self):
        release, _ = load_factory_manifest(self.manifest)
        self.assertEqual(set(release["images"]), set(FACTORY_WRITE_ORDER))

    def test_writes_whole_set_in_order_boot_last_with_readback_and_identity(self):
        self.invoke()
        written = [e.split(":", 1)[1] for e in self.events if e.startswith("write:")]
        self.assertEqual(written, list(FACTORY_WRITE_ORDER))
        self.assertEqual(written[-1], "boot")
        for name in FACTORY_WRITE_ORDER:
            i = self.events.index("write:" + name)
            self.assertEqual(self.events[i + 1], "hash:" + name)
        record = read_json(self.args.journal_dir / "journal.json")
        self.assertTrue(record["complete"] and record["identity_verified_before"]
                        and record["identity_verified_after"] and record["preserved_preloader"])
        self.assertEqual(record["writes"], {name: "verified" for name in FACTORY_WRITE_ORDER})

    def test_userdata_is_optional_and_skipped_when_absent(self):
        release = copy.deepcopy(self.release)
        del release["images"]["userdata"]
        self.manifest.write_text(json.dumps(release))
        loaded, _ = load_factory_manifest(self.manifest)
        self.assertNotIn("userdata", loaded["images"])
        self.invoke()
        written = [e.split(":", 1)[1] for e in self.events if e.startswith("write:")]
        self.assertNotIn("userdata", written)
        self.assertEqual(written[-1], "boot")
        record = read_json(self.args.journal_dir / "journal.json")
        self.assertTrue(record["complete"])
        self.assertNotIn("userdata", record["writes"])

    def test_gates_required_before_any_usb(self):
        for flag in ("allow_private_flash", "confirm_factory_reset"):
            setattr(self.args, flag, False)
            with self.assertRaises(InstallError):
                self.invoke()
            setattr(self.args, flag, True)
        self.assertEqual(self.events, [])

    def test_non_factory_purpose_is_rejected(self):
        self.release["purpose"] = "stock-baseline"
        self.manifest.write_text(json.dumps(self.release))
        with self.assertRaisesRegex(InstallError, "factory-restore"):
            self.invoke()
        self.assertEqual(self.events, [])

    def test_calibration_partition_in_images_is_rejected(self):
        self.release["images"]["nvram"] = self.release["images"]["boot"]
        self.manifest.write_text(json.dumps(self.release))
        with self.assertRaises(InstallError):
            self.invoke()
        self.assertEqual(self.events, [])

    def test_system_vendor_and_boot_are_required(self):
        for name in ("system", "vendor", "boot"):
            release = copy.deepcopy(self.release)
            del release["images"][name]
            self.manifest.write_text(json.dumps(release))
            with self.assertRaisesRegex(InstallError, "system, vendor and boot"):
                self.invoke()
        self.manifest.write_text(json.dumps(self.release))

    def test_sparse_image_is_rejected(self):
        path = self.bundle / "system.img"
        data = bytearray(path.read_bytes())
        data[:4] = b"\x3a\xff\x26\xed"
        path.write_bytes(data)
        self.release["images"]["system"]["sha256"] = hashlib.sha256(data).hexdigest()
        self.manifest.write_text(json.dumps(self.release))
        with self.assertRaisesRegex(InstallError, "sparse"):
            self.invoke()
        self.assertEqual(self.events, [])

    def test_system_without_ext4_magic_is_rejected(self):
        path = self.bundle / "vendor.img"
        data = bytearray(path.read_bytes())
        data[0x438:0x43a] = b"\x00\x00"
        path.write_bytes(data)
        self.release["images"]["vendor"]["sha256"] = hashlib.sha256(data).hexdigest()
        self.manifest.write_text(json.dumps(self.release))
        with self.assertRaisesRegex(InstallError, "ext4"):
            self.invoke()
        self.assertEqual(self.events, [])

    def test_calibration_mismatch_prevents_writer_creation(self):
        self.baseline["identity_sha256"]["nvram"] = "0" * 64
        self.baseline_path.write_text(json.dumps(self.baseline))
        with self.assertRaisesRegex(InstallError, "baseline differs"):
            self.invoke()
        self.assertNotIn("writer-created", self.events)

    def test_failed_write_stops_with_no_boot_and_no_resume(self):
        self.fail = "vendor"
        with self.assertRaisesRegex(InstallError, "write failure"):
            self.invoke()
        record = read_json(self.args.journal_dir / "journal.json")
        self.assertEqual(record["writes"]["system"], "verified")
        self.assertEqual(record["writes"]["vendor"], "writing")
        self.assertFalse(record["complete"])
        self.assertNotIn("write:boot", self.events)
        self.assertNotIn("exit", self.events)
        with self.assertRaisesRegex(InstallError, "new factory-restore journal"):
            self.invoke()


if __name__ == "__main__":
    unittest.main()
