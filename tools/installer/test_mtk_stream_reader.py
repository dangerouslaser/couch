"""Read-protocol experiments with in-memory storage and packet-buffered USB only."""
import array
import hashlib
import struct
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

from couch_install import CHUNK, InstallError
from mtk_readonly import REVIEWED_REVISION
from mtk_stream_reader import ConnectedMtkStreamReader
from mtk_usb import PacketBufferedInput
from test_mtk_readonly import fake_session

ACK = b"\x5a"


class ReadWire:
    wMaxPacketSize = 512

    def __init__(self, disk):
        self.disk = disk
        self.pending = bytearray()
        self.sent = []
        self.requests = []
        self.phase = "idle"
        self.fields = []
        self.offset = 0
        self.remaining = 0
        self.bad_checksum = False
        self.bad_header = False
        self.bad_write = None
        self.short_read = False
        self.extra_checksum = False
        self.zero_packets = 0

    def packet(self):
        size = min(CHUNK, self.remaining)
        data = bytes(self.disk[self.offset:self.offset + size])
        checksum = sum(data) & 0xffff
        if self.bad_checksum:
            checksum ^= 1
        self.pending.extend(data + struct.pack(">H", checksum))
        if self.extra_checksum:
            self.pending.extend(b"\x99")
        self.offset += size
        self.remaining -= size

    def write(self, data, timeout):
        assert timeout == 1000
        data = bytes(data)
        self.sent.append(data)
        if self.bad_write is not None:
            return self.bad_write
        if self.phase == "idle":
            assert data == b"\x72"
            self.pending.extend(ACK + b"\x01")
            self.phase = "switch"
        elif self.phase == "switch":
            assert data == b"\x60"
            self.pending.extend(ACK)
            self.phase = "user"
        elif self.phase == "user":
            assert data == b"\x08"
            self.pending.extend(ACK)
            self.phase = "header"
        elif self.phase == "header":
            self.fields.append(data)
            if len(self.fields) == 6:
                assert self.fields[:3] == [b"\xd6", b"\x0c", b"\x02"]
                assert self.fields[-1] == struct.pack(">I", CHUNK)
                self.offset, self.remaining = map(lambda v: struct.unpack(">Q", v)[0], self.fields[3:5])
                self.fields.clear()
                self.pending.extend(b"\xa5" if self.bad_header else ACK)
                self.packet()  # May share a physical packet with the header ACK.
                self.phase = "data_ack"
        elif self.phase == "data_ack":
            assert data == ACK
            if self.remaining:
                self.packet()
            else:
                self.phase = "idle"
        return len(data)

    def read(self, size, timeout):
        assert timeout == 1000
        self.requests.append(size)
        if self.zero_packets:
            self.zero_packets -= 1
            return array.array("B")
        if self.short_read:
            raise TimeoutError("fixture transfer timeout")
        assert self.pending, "unexpected DA read without response"
        data = bytes(self.pending[:size])
        del self.pending[:size]
        return array.array("B", data)


class StreamReaderTests(unittest.TestCase):
    def setUp(self):
        self.mtk, self.disk, self.gpt_reads = fake_session()
        self.wire = ReadWire(self.disk)
        self.mtk.port = NS(cdc=NS(EP_IN=PacketBufferedInput(self.wire), EP_OUT=self.wire))
        self.reader = ConnectedMtkStreamReader(self.mtk, REVIEWED_REVISION)

    def test_multi_packet_tail_checksum_and_coalesced_header_are_correct(self):
        chunks = list(self.reader.chunks("userdata"))
        self.assertEqual([len(data) for data in chunks], [CHUNK, 512])
        region = self.reader.description["partitions"]["userdata"]
        self.assertEqual(b"".join(chunks), self.disk[region["offset"]:region["offset"] + region["size"]])
        self.assertEqual(self.reader.stats["windows"], 1)
        self.assertEqual(self.reader.stats["packets"], 2)
        self.assertEqual(self.reader.stats["bytes"], CHUNK + 512)
        self.assertEqual(self.wire.sent.count(b"\x72"), 1)
        self.assertEqual(self.wire.sent.count(b"\x60"), 1)
        self.assertEqual(self.wire.sent.count(ACK), 2)
        self.assertLessEqual(max(self.wire.requests), CHUNK)
        self.assertFalse(self.reader.poisoned)

    def test_each_window_reestablishes_header_and_hashes_independently(self):
        with patch("mtk_stream_reader.WINDOW", CHUNK):
            value = self.reader.hash("userdata")
        region = self.reader.description["partitions"]["userdata"]
        self.assertEqual(value, hashlib.sha256(self.disk[region["offset"]:region["offset"] + region["size"]]).hexdigest())
        self.assertEqual(self.wire.sent.count(b"\x72"), 2)
        self.assertEqual(self.wire.sent.count(b"\xd6"), 2)
        self.assertEqual(self.wire.sent.count(b"\x08"), 2)

    def test_checksum_mismatch_does_not_ack_or_yield_untrusted_packet(self):
        self.wire.bad_checksum = True
        with self.assertRaisesRegex(InstallError, "checksum mismatch"):
            next(self.reader.chunks("userdata"))
        self.assertNotIn(ACK, self.wire.sent)
        self.assertTrue(self.reader.poisoned)
        writes = list(self.wire.sent)
        with self.assertRaisesRegex(InstallError, "poisoned"):
            self.reader.hash("boot")
        self.assertEqual(self.wire.sent, writes)

    def test_bad_header_and_short_usb_write_stop_without_retry(self):
        self.wire.bad_header = True
        with self.assertRaisesRegex(InstallError, "not acknowledged"):
            self.reader.hash("boot")
        self.assertTrue(self.reader.poisoned)
        self.setUp()
        self.wire.bad_write = True  # bool success hides short writes and retries.
        with self.assertRaisesRegex(InstallError, "ambiguous"):
            self.reader.hash("boot")
        self.assertEqual(len(self.wire.sent), 1)
        self.assertTrue(self.reader.poisoned)

    def test_legitimate_zero_length_packets_tolerated_but_storm_poisoned(self):
        self.wire.zero_packets = 2
        self.reader.hash("boot")
        self.assertFalse(self.reader.poisoned)
        self.wire.zero_packets = 9
        with self.assertRaisesRegex(InstallError, "zero-length"):
            self.reader.hash("boot")
        self.assertTrue(self.reader.poisoned)

    def test_abandoned_active_window_poisoned_without_draining_or_retry(self):
        stream = self.reader.chunks("userdata")
        next(stream)
        before = (list(self.wire.sent), list(self.wire.requests))
        stream.close()
        self.assertTrue(self.reader.poisoned)
        self.assertEqual((self.wire.sent, self.wire.requests), before)

    def test_completed_window_may_close_without_poisoning(self):
        stream = self.reader.chunks("boot")
        next(stream)
        stream.close()
        self.assertFalse(self.reader.poisoned)
        self.reader.hash("recovery")

    def test_parallel_or_out_of_partition_reads_are_rejected(self):
        for name in ("../boot", "gpt", "missing"):
            with self.assertRaisesRegex(InstallError, "Unknown"):
                self.reader.hash(name)
        with self.assertRaisesRegex(InstallError, "partition-bound"):
            self.reader._read(512, 512)
        self.assertEqual(self.wire.sent, [])
        stream = self.reader.chunks("userdata")
        next(stream)
        with self.assertRaisesRegex(InstallError, "active"):
            self.reader.hash("boot")
        stream.close()

    def test_observed_layout_cannot_be_modified_to_expand_read(self):
        self.reader.description["partitions"]["boot"]["size"] += CHUNK
        with self.assertRaisesRegex(InstallError, "modified"):
            self.reader.hash("boot")
        self.assertEqual(self.wire.sent, [])

    def test_timeout_and_surplus_after_checksum_poison(self):
        self.wire.short_read = True
        with self.assertRaisesRegex(InstallError, "Ambiguous"):
            self.reader.hash("boot")
        self.assertTrue(self.reader.poisoned)
        self.setUp()
        self.wire.extra_checksum = True
        with self.assertRaisesRegex(InstallError, "Unexpected bytes"):
            self.reader.hash("boot")
        self.assertTrue(self.reader.poisoned)


if __name__ == "__main__":
    unittest.main()
