"""Experimental partition streaming reader; not wired into the physical CLI.

Pinned mtkclient 60e07f3b343a4469389f15967626d63e049968d4 legacy readflash
repeats speed query, partition select and READ header for every adapter chunk.
Its memory loop also reverses the remaining-byte subtraction after one packet.
This prototype retains 1 MiB DA packets but amortizes headers over 64 MiB windows.
It has not been benchmarked on hardware. Read sum16 checksum semantics must be
confirmed on a small partition in a fresh read-only session before deployment;
upstream reads that checksum but does not compare it. No serial baud adjustment,
USB re-enumeration, storage mutation, or concurrent-session access is exposed.
"""
import time
import struct

from couch_install import CHUNK, InstallError, require
from mtk_readonly import ConnectedMtkReader
from mtk_usb import bounded_operation

WINDOW = 64 * CHUNK
ACK = b"\x5a"


class ConnectedMtkStreamReader(ConnectedMtkReader):
    """Already-bound legacy session; GPT/identity discovery remains the reader's.

    Only chunks/hash/backup_identity are intended public I/O. Always exhaust or
    close a chunks iterator before another read. Abandonment while a DA window
    is active poisons the session without issuing cleanup commands. A fresh
    externally established session is required after any ambiguous protocol I/O.
    """
    def __init__(self, mtk, revision):
        self.poisoned = False
        self._active = False
        self._ready = False
        super().__init__(mtk, revision)
        self._ep_in = mtk.port.cdc.EP_IN
        self._ep_out = mtk.port.cdc.EP_OUT
        self.description["transport"] = "mtkclient-connected-experimental-stream-reader"
        self._regions = {name: dict(region) for name, region in self.description["partitions"].items()}
        self.stats = {"windows": 0, "packets": 0, "bytes": 0,
                      "header_seconds": 0.0, "payload_seconds": 0.0}
        self._ready = True

    def _healthy(self):
        require(not self.poisoned, "Streaming read session is poisoned; do not retry")
        require(not self._active, "Another partition stream is active; exhaust or close it first")

    def _read(self, offset, size):
        # During construction the established backend owns readflash's deadline;
        # both GPT CRCs are checked by ConnectedMtkReader before streaming starts.
        self._healthy()
        require(not self._ready, "Use partition-bound streaming reads, not arbitrary byte ranges")
        try:
            return super()._read(offset, size)
        except BaseException:
            self.poisoned = True
            raise

    def _send(self, data):
        result = self._ep_out.write(data, timeout=1000)
        require(type(result) is int and result == len(data), "Short or ambiguous read-protocol USB write")

    def _receive(self, count):
        data = self._ep_in.read(count, timeout=1000)
        require(isinstance(data, (bytes, bytearray)) and len(data) == count,
                "Short or malformed read-protocol USB response")
        return bytes(data)

    def _ack(self):
        require(self._receive(1) == ACK, "Read-protocol command was not acknowledged")

    def _header(self, offset, size):
        # Preserve the known DA negotiation, but perform it once per window.
        self._send(b"\x72")  # USB_CHECK_STATUS; reports current speed, no speed change.
        self._ack()
        self._receive(1)  # Speed byte; endpoints already perform full-size reads.
        self._send(b"\x60")  # SDMMC_SWITCH_PART_CMD
        self._ack()
        self._send(b"\x08")  # EMMC_PART_USER; never boot/RPMB/GP areas.
        self._ack()
        for field in (b"\xd6", b"\x0c", b"\x02", struct.pack(">Q", offset),
                      struct.pack(">Q", size), struct.pack(">I", CHUNK)):
            self._send(field)
        self._ack()
        # The DA may already have sent payload bytes alongside its ACK. Keep
        # PacketBufferedInput's surplus; do not enforce an empty buffer here.

    def chunks(self, name):
        self._healthy()
        require(name in self._regions, "Unknown partition")
        require(self.description["partitions"] == self._regions, "Observed partition layout was modified")
        region = self._regions[name]
        require(type(region["offset"]) is int and type(region["size"]) is int
                and region["offset"] >= 512 and region["size"] > 0
                and region["offset"] % 512 == 0 and region["size"] % 512 == 0
                and region["offset"] + region["size"] <= self.capacity,
                "Partition exceeds bound user storage")
        for start in range(0, region["size"], WINDOW):
            self._healthy()
            size = min(WINDOW, region["size"] - start)
            self._active = True
            try:
                at = time.monotonic()
                with bounded_operation(10):
                    self._header(region["offset"] + start, size)
                self.stats["header_seconds"] += time.monotonic() - at
                self.stats["windows"] += 1
                remaining = size
                while remaining:
                    count = min(CHUNK, remaining)
                    at = time.monotonic()
                    with bounded_operation(10):
                        data = self._receive(count)
                        checksum = struct.unpack(">H", self._receive(2))[0]
                        require(checksum == sum(data) & 0xffff, "Streaming read checksum mismatch")
                        # Next data is gated by our ACK. Any pre-ACK surplus
                        # after checksum is not a valid next packet.
                        require(not getattr(self._ep_in, "pending", b""), "Unexpected bytes after read checksum")
                        self._send(ACK)
                    self.stats["payload_seconds"] += time.monotonic() - at
                    self.stats["packets"] += 1
                    self.stats["bytes"] += count
                    remaining -= count  # Correct sign, independent of accumulated buffer length.
                    if remaining == 0:
                        self._active = False
                    yield data
            except BaseException as error:
                if self._active or not isinstance(error, GeneratorExit):
                    self.poisoned = True
                self._active = False
                if isinstance(error, Exception) and not isinstance(error, InstallError):
                    raise InstallError("Ambiguous streaming read; discard the session") from error
                raise
