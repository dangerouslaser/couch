"""Private, bounded MT6580 legacy DA writer; no USB discovery or public CLI.

Protocol reviewed from mtkclient 60e07f3b343a4469389f15967626d63e049968d4,
Library/DA/legacy/dalegacy_lib.py:sdmmc_write_data. Do not use its writeflash:
its USB wrapper retries ambiguous writes and accepts padding/truncated sources.
The caller must establish an exclusive pinned session and independently validate
model, identity backups and layout before constructing this facade. Construction
and successful DA acknowledgements are not installation/readback certification.
"""
import copy
import hashlib
import os
from pathlib import Path
import re
import stat
import struct

from couch_install import CHUNK, MODEL, InstallError, WRITE_ORDER, fingerprint, layout, require, sync_directory
from mtk_readonly import ConnectedMtkReader
from mtk_usb import bounded_operation

COMMAND = b"\x62"
ACK = b"\x5a"
CONT = b"\x69"
PROGRESS_INTERVAL = 64 * 1024 * 1024
EMMC_USER = b"\x01\x08"  # MTK_DA_STORAGE_EMMC, EMMC_PART_USER; never boot regions.


def _stamp(info):
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns


def _open_image(path):
    # Native sessions provide a private parent. Check the opened object as well
    # as the name: Windows has no O_NOFOLLOW and binary mode is not the default.
    before = path.lstat()
    require(stat.S_ISREG(before.st_mode) and not
            (getattr(before, "st_file_attributes", 0) & 0x400), "Image is a link or reparse point")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_BINARY", 0)
    fd = os.open(path, flags)
    try:
        require(_stamp(os.fstat(fd)) == _stamp(before), "Image changed while opening")
        os.set_inheritable(fd, False)
        return fd
    except BaseException:
        os.close(fd)
        raise


def _sha(value):
    return isinstance(value, str) and re.fullmatch(r"[a-f0-9]{64}", value) is not None


class ConnectedMtkWriter(ConnectedMtkReader):
    """A single-session facade compatible with the install transaction engine.

    binding must contain model, model_verified=True, storage_id,
    runtime_cid_sha256, partitions and identity_sha256. The last is the digest
    of the independently prevalidated identity record, not a vendor ID inferred
    from CID. This adapter compares the observed fields; the caller owns proof
    of the model and decoded identity. A separate verified originals manifest
    can construct a fresh writer for restoration; no arbitrary-source override.

    write() only transmits. The transaction engine must independently hash the
    complete partition after each call and verify identity again at completion.
    After any ambiguous I/O, discard the session; even hash() is then forbidden.
    """
    def __init__(self, mtk, revision, *, release, bundle, binding, progress=None):
        require(progress is None or callable(progress), "Invalid progress callback")
        self._progress = progress
        self.poisoned = False
        self._closed = False
        self._sources = {}
        self._first_write = True
        try:
            super().__init__(mtk, revision)
            require(isinstance(binding, dict) and binding.get("model") == MODEL
                    and binding.get("model_verified") is True and _sha(binding.get("identity_sha256")),
                    "Explicit independently prevalidated model/identity binding required")
            for key in ("storage_id", "runtime_cid_sha256", "partitions"):
                require(binding.get(key) == self.description[key], f"Observed {key} differs from identity binding")
            self._binding = copy.deepcopy(binding)
            self._release = copy.deepcopy(release)
            require(self._release.get("schema") == 1 and self._release.get("model") == MODEL,
                    "Unsupported verified manifest")
            require(layout(self._release["partitions"]) == self.description["partitions"],
                    "Verified manifest layout differs from observed GPT")
            images = self._release.get("images")
            require(isinstance(images, dict) and images and images.keys() <= set(WRITE_ORDER),
                    "Manifest contains a prohibited write target")
            self._bundle = Path(bundle).resolve(strict=True)
            require(self._bundle.is_dir(), "Missing verified bundle directory")
            for name, image in images.items():
                filename = image.get("file")
                require(isinstance(filename, str) and Path(filename).name == filename
                        and filename not in ("", ".", "..") and _sha(image.get("sha256")),
                        "Invalid verified image entry")
                path = self._bundle / filename
                fd = _open_image(path)
                self._sources[name] = {"path": path, "fd": fd, "sha256": image["sha256"]}
                info = os.fstat(fd)
                require(stat.S_ISREG(info.st_mode) and info.st_size == self.description["partitions"][name]["size"],
                        f"Expected raw full-partition regular image: {name}")
                self._sources[name]["stamp"] = _stamp(info)
            self._validate_all()
            # Read exact endpoint counts directly: upstream usbwrite's bool
            # hides retries and partial transfers. Keep the buffered DA input.
            self._ep_out = mtk.port.cdc.EP_OUT
            self._ep_in = mtk.port.cdc.EP_IN
            require(callable(self._ep_out.write) and callable(self._ep_in.read), "Missing bound USB endpoints")
            self.description.update(model=MODEL, model_verified=True,
                                    transport="mtkclient-connected-private-writer",
                                    identity_sha256=binding["identity_sha256"])
            self._description_hash = fingerprint(self.description)
        except BaseException:
            self.close()
            raise

    def _report(self, phase, name, completed, total):
        if completed not in (0, total) and completed % PROGRESS_INTERVAL:
            return
        if self._progress is not None:
            self._progress(phase, name, completed, total)
        else:
            print(f"{phase}: {name} {completed}/{total} bytes ({completed * 100 // total}%)", flush=True)

    def _healthy(self):
        require(not self.poisoned and not self._closed, "Writer session is poisoned or closed; do not retry")

    def _read(self, offset, size):
        self._healthy()
        try:
            # ExactUsbBackend owns the readflash deadline. Nesting alarm
            # contexts would reject the already bounded physical reader.
            return super()._read(offset, size)
        except BaseException:
            self.poisoned = True
            raise

    def _unchanged(self, name):
        source = self._sources[name]
        require(_stamp(os.fstat(source["fd"])) == source["stamp"], f"Verified image changed: {name}")
        info = source["path"].lstat()
        require(stat.S_ISREG(info.st_mode) and _stamp(info) == source["stamp"],
                f"Verified image path changed: {name}")

    def _validate_source(self, name):
        self._unchanged(name)
        source = self._sources[name]
        full = hashlib.sha256()
        chunks = []
        total = source["stamp"][2]
        self._report("Verify image", name, 0, total)
        for offset in range(0, source["stamp"][2], CHUNK):
            count = min(CHUNK, source["stamp"][2] - offset)
            data = os.pread(source["fd"], count, offset)
            require(len(data) == count, f"Short verified image: {name}")
            full.update(data)
            chunks.append(hashlib.sha256(data).digest())
            if offset + count < total:
                self._report("Verify image", name, offset + count, total)
        require(full.hexdigest() == source["sha256"], f"Verified image SHA-256 mismatch: {name}")
        self._unchanged(name)
        source["chunks"] = chunks
        self._report("Verify image", name, total, total)

    def _validate_all(self):
        for name in self._sources:
            self._validate_source(name)

    def _send(self, data):
        result = self._ep_out.write(data, timeout=1000)
        require(type(result) is int and result == len(data), "Short or ambiguous USB write; session poisoned")

    def _expect(self, expected):
        result = self._ep_in.read(1, timeout=1000)
        require(isinstance(result, (bytes, bytearray)) and bytes(result) == expected,
                "Unexpected DA acknowledgement; session poisoned")
        # The legacy write protocol has one response byte per step, never a
        # pipelined response. Do not silently consume stale surplus as the next ACK.
        require(not getattr(self._ep_in, "pending", b""), "Unexpected buffered DA bytes; session poisoned")

    def write(self, name, source):
        self._healthy()
        require(name in WRITE_ORDER and name in self._sources, "Prohibited or unmanifested write target")
        require(Path(source).absolute().parent.resolve() / Path(source).name == self._sources[name]["path"],
                "Write source differs from verified manifest")
        require(fingerprint(self.description) == self._description_hash, "Bound device description changed")
        # Revalidate every image after potentially long backup work, before the
        # first write. Then rehash this image before each later partition write.
        if self._first_write:
            self._validate_all()
        else:
            self._validate_source(name)
        region = self.description["partitions"][name]
        require(region["offset"] % 512 == 0 and region["size"] % 512 == 0
                and region["offset"] + region["size"] <= self.capacity, "Write exceeds bound user storage")
        image = self._sources[name]
        self._report("Write", name, 0, region["size"])
        try:
            with bounded_operation(10):
                for field in (COMMAND, EMMC_USER[:1], EMMC_USER[1:], struct.pack(">Q", region["offset"]),
                              struct.pack(">Q", region["size"]), struct.pack(">I", CHUNK)):
                    self._send(field)
                self._expect(ACK)
            for index, offset in enumerate(range(0, region["size"], CHUNK)):
                self._unchanged(name)
                count = min(CHUNK, region["size"] - offset)
                data = os.pread(image["fd"], count, offset)
                require(len(data) == count and hashlib.sha256(data).digest() == image["chunks"][index],
                        f"Image changed during transfer: {name}")
                with bounded_operation(10):
                    self._send(ACK)
                    self._send(data)
                    self._send(struct.pack(">H", sum(data) & 0xffff))
                    self._expect(CONT)
                if offset + count < region["size"]:
                    self._report("Write", name, offset + count, region["size"])
            self._unchanged(name)
            self._report("Write", name, region["size"], region["size"])
            self._first_write = False
        except BaseException as error:
            self.poisoned = True
            if isinstance(error, Exception) and not isinstance(error, InstallError):
                raise InstallError("Ambiguous MTK write; session poisoned, keep backups and journal") from error
            raise

    def backup(self, name, destination):
        self._healthy()
        destination = Path(destination)
        require(name in self.description["partitions"], "Unknown backup partition")
        total = self.description["partitions"][name]["size"]
        completed = 0
        self._report("Backup", name, completed, total)
        fd = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as output:
            for data in self.chunks(name):
                require(output.write(data) == len(data), "Short backup file write")
                completed += len(data)
                if completed < total:
                    self._report("Backup", name, completed, total)
            output.flush()
            os.fsync(output.fileno())
        sync_directory(destination.parent)
        self._report("Backup", name, total, total)

    def hash(self, name):
        self._healthy()
        require(name in self.description["partitions"], "Unknown readback partition")
        total = self.description["partitions"][name]["size"]
        completed = 0
        result = hashlib.sha256()
        self._report("Hash readback", name, completed, total)
        for data in self.chunks(name):
            result.update(data)
            completed += len(data)
            if completed < total:
                self._report("Hash readback", name, completed, total)
        self._report("Hash readback", name, total, total)
        return result.hexdigest()

    def close(self):
        self._closed = True
        for source in self._sources.values():
            if source["fd"] is not None:
                os.close(source["fd"])
                source["fd"] = None

    def __enter__(self):
        self._healthy()
        return self

    def __exit__(self, *args):
        self.close()
