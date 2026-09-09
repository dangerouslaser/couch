from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace as NS
import unittest
import zlib

from couch_install import CHUNK, IDENTITY_PARTITIONS, InstallError, read_json
from mtk_readonly import ConnectedMtkReader, REVIEWED_REVISION


def fake_session():
    capacity = 4 * CHUNK
    disk = bytearray(capacity)
    entries = bytearray(128 * 128)
    offset = 34
    for index, name in enumerate(sorted(IDENTITY_PARTITIONS | {"boot", "recovery", "userdata"})):
        size = 4096 if name != "userdata" else CHUNK + 512
        start = index * 128
        entries[start:start + 16] = b"t" * 16
        entries[start + 16:start + 32] = bytes([index + 1]) * 16
        struct.pack_into("<QQ", entries, start + 32, offset, offset + size // 512 - 1)
        encoded = name.encode("utf-16-le")
        entries[start + 56:start + 56 + len(encoded)] = encoded
        disk[offset * 512:offset * 512 + size] = bytes([index + 1]) * size
        offset += size // 512
    sectors = capacity // 512
    for lba, other, table in ((1, sectors - 1, 2), (sectors - 1, 1, sectors - 33)):
        header = bytearray(512)
        header[:8] = b"EFI PART"
        struct.pack_into("<IIIIQQQQ", header, 8, 0x10000, 92, 0, 0, lba, other, 34, sectors - 34)
        header[56:72] = b"g" * 16
        struct.pack_into("<QIII", header, 72, table, 128, 128, zlib.crc32(entries))
        struct.pack_into("<I", header, 16, zlib.crc32(header[:92]))
        disk[lba * 512:(lba + 1) * 512] = header
        disk[table * 512:table * 512 + len(entries)] = entries
    calls = []

    def readflash(**kwargs):
        calls.append(kwargs)
        return bytes(disk[kwargs["addr"]:kwargs["addr"] + kwargs["length"]])

    da = NS(storage=NS(flashtype="emmc"),
            legacy_storage=NS(emmc=NS(m_emmc_ua_size=capacity, m_emmc_cid=[123, 456])))
    return NS(config=NS(hwcode=0x6580), daloader=NS(daconfig=da, readflash=readflash)), disk, calls


class MtkReaderTests(unittest.TestCase):
    def setUp(self):
        self.session, self.disk, self.calls = fake_session()

    def reader(self):
        return ConnectedMtkReader(self.session, REVIEWED_REVISION)

    def test_observes_both_gpt_copies_without_claiming_model(self):
        reader = self.reader()
        self.assertFalse(reader.description["model_verified"])
        self.assertEqual(reader.description["capacity"], len(self.disk))
        self.assertEqual(set(reader.description["partitions"]), IDENTITY_PARTITIONS | {"boot", "recovery", "userdata"})
        self.assertTrue(all(call["parttype"] == "user" and call["filename"] == "" for call in self.calls))
        self.assertFalse(hasattr(reader, "write"))

    def test_large_partition_is_read_in_bounded_packets(self):
        reader = self.reader()
        self.calls.clear()
        chunks = list(reader.chunks("userdata"))
        self.assertEqual([len(chunk) for chunk in chunks], [CHUNK, 512])
        self.assertTrue(all(call["length"] <= CHUNK for call in self.calls))

    def test_short_read_stops(self):
        self.session.daloader.readflash = lambda **kwargs: b""
        with self.assertRaisesRegex(InstallError, "Short or failed"):
            self.reader()

    def test_header_and_array_corruption_are_rejected(self):
        for position, message in ((512 + 56, "header CRC"), (1024 + 56, "entry CRC"),
                                  (len(self.disk) - 512 + 56, "header CRC")):
            with self.subTest(position=position):
                self.disk[position] ^= 1
                with self.assertRaisesRegex(InstallError, message):
                    self.reader()
                self.disk[position] ^= 1

    def test_individually_valid_but_different_gpt_copies_are_rejected(self):
        header_start = len(self.disk) - 512
        table_start = len(self.disk) - 33 * 512
        self.disk[table_start + 56] = ord("z")
        struct.pack_into("<I", self.disk, header_start + 88,
                         zlib.crc32(self.disk[table_start:table_start + 16384]))
        struct.pack_into("<I", self.disk, header_start + 16, 0)
        struct.pack_into("<I", self.disk, header_start + 16,
                         zlib.crc32(self.disk[header_start:header_start + 92]))
        with self.assertRaisesRegex(InstallError, "Primary and backup GPT differ"):
            self.reader()

    def test_wrong_chip_revision_or_storage_refused_before_read(self):
        with self.assertRaisesRegex(InstallError, "revision"):
            ConnectedMtkReader(self.session, "other")
        self.session.config.hwcode = 0x1234
        with self.assertRaisesRegex(InstallError, "MT6580"):
            self.reader()
        self.assertEqual(self.calls, [])

    def test_backup_is_private_and_independently_verified(self):
        reader = self.reader()
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "originals"
            report = reader.backup_identity(target, reader.description["storage_id"])
            self.assertTrue(report["complete"])
            self.assertFalse(report["identity_decoded"])
            self.assertEqual(set(report["backups"]), IDENTITY_PARTITIONS)
            self.assertEqual(target.stat().st_mode & 0o777, 0o700)
            for path in target.iterdir():
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_backup_mismatch_never_marks_complete(self):
        reader = self.reader()
        reader.hash = lambda name: "0" * 64
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "originals"
            with self.assertRaisesRegex(InstallError, "readback mismatch"):
                reader.backup_identity(target, reader.description["storage_id"])
            self.assertFalse(read_json(target / "readback.json")["complete"])

    def test_target_confirmation_precedes_destination_creation(self):
        reader = self.reader()
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "originals"
            with self.assertRaisesRegex(InstallError, "confirmation"):
                reader.backup_identity(target, "other")
            self.assertFalse(target.exists())

    def test_transfer_bounds_and_unknown_partition(self):
        reader = self.reader()
        for offset, length in ((-512, 512), (0, CHUNK + 512), (0, 513), (len(self.disk), 512)):
            with self.assertRaisesRegex(InstallError, "bounds"):
                reader._read(offset, length)
        with self.assertRaisesRegex(InstallError, "Unknown partition"):
            list(reader.chunks("invented"))


if __name__ == "__main__":
    unittest.main()
