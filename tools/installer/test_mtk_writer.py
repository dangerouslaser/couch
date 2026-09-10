"""In-memory wire tests only: never discover or claim USB."""
import array
import copy
import hashlib
import struct
import tempfile
from pathlib import Path
from types import SimpleNamespace as NS
import unittest

from couch_install import CHUNK, IDENTITY_PARTITIONS, InstallError, MODEL, install_transaction
from mtk_readonly import ConnectedMtkReader, REVIEWED_REVISION
from mtk_writer import ACK, COMMAND, CONT, PROGRESS_INTERVAL, ConnectedMtkWriter
from mtk_usb import PacketBufferedInput
from test_mtk_readonly import fake_session


class Wire:
    def __init__(self, disk):
        self.disk = disk
        self.writes = []
        self.responses = []
        self.fields = []
        self.phase = "header"
        self.data = None
        self.count = 0
        self.fail_at = None
        self.result = None
        self.bad_ack = False
        self.corrupt = False
        self.hook = None

    def write(self, data, timeout):
        assert timeout == 1000
        data = bytes(data)
        self.writes.append(data)
        if self.fail_at == len(self.writes):
            if isinstance(self.result, BaseException):
                raise self.result
            return self.result
        if self.phase == "header":
            self.fields.append(data)
            if len(self.fields) == 6:
                assert self.fields[:3] == [COMMAND, b"\x01", b"\x08"]
                self.offset = struct.unpack(">Q", self.fields[3])[0]
                self.length = struct.unpack(">Q", self.fields[4])[0]
                assert self.fields[5] == struct.pack(">I", CHUNK)
                self.fields.clear()
                self.responses.append(ACK)
                self.phase = "ack"
                self.count = 0
        elif self.phase == "ack":
            assert data == ACK
            self.phase = "data"
        elif self.phase == "data":
            assert len(data) == min(CHUNK, self.length - self.count)
            self.data = data
            self.phase = "checksum"
        else:
            assert data == struct.pack(">H", sum(self.data) & 0xffff)
            start = self.offset + self.count
            self.disk[start:start + len(self.data)] = self.data
            if self.corrupt:
                self.disk[start] ^= 1
            self.count += len(self.data)
            self.phase = "header" if self.count == self.length else "ack"
            self.responses.append(CONT)
        if self.hook:
            self.hook(len(self.writes))
        return len(data)

    def read(self, size, timeout):
        assert size == 1 and timeout == 1000
        reply = self.responses.pop(0)
        return b"\xa5" if self.bad_ack else reply


class MtkWriterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.bundle = Path(self.temp.name) / "release"
        self.bundle.mkdir()
        self.mtk, self.disk, self.reads = fake_session()
        self.reader = ConnectedMtkReader(self.mtk, REVIEWED_REVISION)
        self.wire = Wire(self.disk)
        self.mtk.port = NS(cdc=NS(EP_OUT=self.wire, EP_IN=self.wire))
        self.binding = {key: copy.deepcopy(self.reader.description[key]) for key in
                        ("storage_id", "runtime_cid_sha256", "partitions")}
        self.binding.update(model=MODEL, model_verified=True, identity_sha256="a" * 64)
        self.release = {"schema": 1, "model": MODEL,
                        "partitions": copy.deepcopy(self.binding["partitions"]), "images": {}}
        for i, name in enumerate(("recovery", "userdata", "boot")):
            data = bytes([81 + i]) * self.binding["partitions"][name]["size"]
            filename = f"{name}.img"
            (self.bundle / filename).write_bytes(data)
            self.release["images"][name] = {"file": filename, "sha256": hashlib.sha256(data).hexdigest()}

    def writer(self, **kwargs):
        writer = ConnectedMtkWriter(self.mtk, REVIEWED_REVISION, release=kwargs.get("release", self.release),
                                     bundle=self.bundle, binding=kwargs.get("binding", self.binding),
                                     progress=kwargs.get("progress", lambda *args: None))
        self.addCleanup(writer.close)
        return writer

    def test_actual_packet_buffer_handles_array_responses_and_rejects_surplus(self):
        wire = self.wire
        class RawInput:
            wMaxPacketSize = 512
            surplus = b""
            def read(self, size, timeout):
                self_size = size
                assert self_size == 512
                return array.array("B", wire.read(1, timeout) + self.surplus)
        endpoint = RawInput()
        self.mtk.port.cdc.EP_IN = PacketBufferedInput(endpoint)
        writer = self.writer()
        writer.write("boot", self.bundle / "boot.img")
        self.assertEqual(writer.hash("boot"), self.release["images"]["boot"]["sha256"])
        self.assertEqual(self.mtk.port.cdc.EP_IN.pending, b"")
        endpoint.surplus = CONT
        with self.assertRaisesRegex(InstallError, "buffered"):
            writer.write("boot", self.bundle / "boot.img")
        self.assertTrue(writer.poisoned)

    def test_progress_reports_bounded_intervals_and_completed_operations(self):
        reports = []
        writer = self.writer(progress=lambda *args: reports.append(args))
        self.assertTrue(all(phase == "Verify image" for phase, *_ in reports))
        reports.clear()
        total = 2 * PROGRESS_INTERVAL + CHUNK
        for done in range(0, total + 1, CHUNK):
            writer._report("Write", "userdata", done, total)
        self.assertEqual([r[2] for r in reports], [0, PROGRESS_INTERVAL, 2 * PROGRESS_INTERVAL, total])
        reports.clear()
        writer.backup("boot", Path(self.temp.name) / "progress-backup.img")
        writer.hash("boot")
        writer.write("boot", self.bundle / "boot.img")
        for phase in ("Backup", "Hash readback", "Write"):
            matching = [r for r in reports if r[0] == phase]
            self.assertEqual([r[2] for r in matching], [0, self.binding["partitions"]["boot"]["size"]])

    def test_exact_header_chunk_checksum_and_independent_readback(self):
        writer = self.writer()
        writer.write("userdata", self.bundle / "userdata.img")
        region = self.binding["partitions"]["userdata"]
        self.assertEqual(self.wire.writes[:6], [COMMAND, b"\x01", b"\x08", struct.pack(">Q", region["offset"]),
                                               struct.pack(">Q", CHUNK + 512), struct.pack(">I", CHUNK)])
        self.assertEqual([len(self.wire.writes[i]) for i in (7, 10)], [CHUNK, 512])
        self.assertEqual(self.wire.writes[8], struct.pack(">H", sum(self.wire.writes[7]) & 0xffff))
        self.reads.clear()
        self.assertEqual(writer.hash("userdata"), self.release["images"]["userdata"]["sha256"])
        self.assertEqual([call["length"] for call in self.reads], [CHUNK, 512])
        self.assertFalse(writer.poisoned)

    def test_every_image_revalidated_before_any_write(self):
        writer = self.writer()
        (self.bundle / "boot.img").write_bytes(b"bad")
        with self.assertRaisesRegex(InstallError, "changed"):
            writer.write("recovery", self.bundle / "recovery.img")
        self.assertEqual(self.wire.writes, [])

    def test_manifest_binding_and_dangerous_targets_fail_without_usb_writes(self):
        for key, value in (("storage_id", "b" * 64), ("runtime_cid_sha256", "b" * 64),
                           ("model_verified", False), ("identity_sha256", "bad")):
            binding = copy.deepcopy(self.binding)
            binding[key] = value
            with self.subTest(key=key), self.assertRaises(InstallError):
                self.writer(binding=binding)
        for name in ("preloader", "lk", "nvram", "proinfo", "misc", "gpt", "cache"):
            release = copy.deepcopy(self.release)
            release["images"][name] = release["images"]["boot"]
            with self.subTest(name=name), self.assertRaises(InstallError):
                self.writer(release=release)
        self.assertEqual(self.wire.writes, [])

    def test_no_arbitrary_sources_or_names(self):
        writer = self.writer()
        for name in ("lk", "misc", "logo", "../boot"):
            with self.assertRaises(InstallError):
                writer.write(name, self.bundle / "boot.img")
        replacement = Path(self.temp.name) / "replacement.img"
        replacement.write_bytes((self.bundle / "boot.img").read_bytes())
        with self.assertRaises(InstallError):
            writer.write("boot", replacement)
        self.assertEqual(self.wire.writes, [])

    def test_size_hash_symlink_and_path_replacement_rejected(self):
        for mode in ("size", "hash", "symlink"):
            path = self.bundle / "boot.img"
            original = path.read_bytes()
            if mode == "size":
                path.write_bytes(original[:-1])
            elif mode == "hash":
                path.write_bytes(b"x" + original[1:])
            else:
                other = self.bundle / "other.img"
                other.write_bytes(original)
                path.unlink()
                path.symlink_to(other)
            with self.subTest(mode=mode), self.assertRaises((InstallError, OSError)):
                self.writer()
            path.unlink()
            path.write_bytes(original)
        writer = self.writer()
        path = self.bundle / "boot.img"
        original = path.read_bytes()
        path.unlink()
        path.write_bytes(original)
        with self.assertRaisesRegex(InstallError, "changed"):
            writer.write("boot", path)
        self.assertEqual(self.wire.writes, [])

    def test_failed_short_or_boolean_usb_result_poison_without_retry(self):
        for bad in (False, True, None, 0, -1, RuntimeError("ambiguous")):
            for phase in (1, 7, 8, 9):
                with self.subTest(bad=bad, phase=phase):
                    self.wire = Wire(self.disk)
                    self.mtk.port.cdc = NS(EP_OUT=self.wire, EP_IN=self.wire)
                    writer = self.writer()
                    self.wire.fail_at, self.wire.result = phase, bad
                    with self.assertRaises(InstallError):
                        writer.write("boot", self.bundle / "boot.img")
                    self.assertTrue(writer.poisoned)
                    count = len(self.wire.writes)
                    with self.assertRaises(InstallError):
                        writer.write("boot", self.bundle / "boot.img")
                    with self.assertRaises(InstallError):
                        writer.hash("boot")
                    self.assertEqual(len(self.wire.writes), count)

    def test_short_payload_and_missing_or_oversized_ack_poison(self):
        writer = self.writer()
        self.wire.fail_at, self.wire.result = 8, 512
        with self.assertRaisesRegex(InstallError, "ambiguous USB write"):
            writer.write("boot", self.bundle / "boot.img")
        self.assertTrue(writer.poisoned)
        for reply in (b"", ACK + CONT, None):
            self.wire = Wire(self.disk)
            self.mtk.port.cdc = NS(EP_OUT=self.wire, EP_IN=NS(read=lambda *args, **kwargs: reply))
            writer = self.writer()
            with self.assertRaisesRegex(InstallError, "acknowledgement"):
                writer.write("boot", self.bundle / "boot.img")
            self.assertTrue(writer.poisoned)
            self.assertEqual(len(self.wire.writes), 6)

    def test_interrupt_after_header_poison_prevents_resume_in_same_session(self):
        writer = self.writer()
        self.wire.fail_at, self.wire.result = 8, KeyboardInterrupt()
        with self.assertRaises(KeyboardInterrupt):
            writer.write("boot", self.bundle / "boot.img")
        self.assertTrue(writer.poisoned)
        with self.assertRaises(InstallError):
            writer.backup("boot", Path(self.temp.name) / "late.img")

    def test_bad_ack_or_mid_transfer_source_change_poison(self):
        writer = self.writer()
        self.wire.bad_ack = True
        with self.assertRaisesRegex(InstallError, "acknowledgement"):
            writer.write("boot", self.bundle / "boot.img")
        self.assertTrue(writer.poisoned)
        self.wire = Wire(self.disk)
        self.mtk.port.cdc = NS(EP_OUT=self.wire, EP_IN=self.wire)
        writer = self.writer()
        def change_after_header(count):
            if count == 6:
                with (self.bundle / "userdata.img").open("r+b") as out:
                    out.write(b"x")
        self.wire.hook = change_after_header
        with self.assertRaisesRegex(InstallError, "changed"):
            writer.write("userdata", self.bundle / "userdata.img")
        self.assertTrue(writer.poisoned)
        self.assertEqual(len(self.wire.writes), 6)

    def test_transaction_backups_order_identity_and_readback(self):
        writer = self.writer()
        originals = {name: writer.hash(name) for name in IDENTITY_PARTITIONS}
        backup = Path(self.temp.name) / "backup"
        install_transaction(self.release, self.bundle, writer, backup,
                          {"device_id": "fixture"}, writer.description["storage_id"])
        self.assertEqual([self.wire.writes[i + 3] for i, value in enumerate(self.wire.writes)
                          if value == COMMAND and i + 5 < len(self.wire.writes)],
                         [struct.pack(">Q", self.binding["partitions"][n]["offset"])
                          for n in ("recovery", "userdata", "boot")])
        self.assertEqual(originals, {name: writer.hash(name) for name in IDENTITY_PARTITIONS})
        self.assertTrue((backup / "journal.json").exists())

    def test_ack_is_not_claimed_as_readback_verification(self):
        writer = self.writer()
        self.wire.corrupt = True
        writer.write("boot", self.bundle / "boot.img")
        self.assertNotEqual(writer.hash("boot"), self.release["images"]["boot"]["sha256"])

    def test_failed_read_poison_and_closed_writer_stops(self):
        writer = self.writer()
        writer._readflash = lambda **kwargs: b""
        with self.assertRaises(InstallError):
            writer.hash("boot")
        self.assertTrue(writer.poisoned)
        writer = self.writer()
        writer.close()
        with self.assertRaises(InstallError):
            writer.write("boot", self.bundle / "boot.img")
        self.assertEqual(self.wire.writes, [])


if __name__ == "__main__":
    unittest.main()
