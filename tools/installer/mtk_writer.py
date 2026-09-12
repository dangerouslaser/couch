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

from couch_install import (CHUNK, MODEL, InstallError, allowed_write_targets, fingerprint,
                           layout, require, sync_directory)
from mtk_readonly import ConnectedMtkReader
from mtk_usb import bounded_operation

COMMAND = b"\x62"
ACK = b"\x5a"
CONT = b"\x69"
PROGRESS_INTERVAL = 64 * 1024 * 1024
EMMC_USER = b"\x01\x08"  # MTK_DA_STORAGE_EMMC, EMMC_PART_USER; never boot regions.


def _stamp(info):
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns


def _windows_information(handle):
    import ctypes
    from ctypes import wintypes

    class Information(ctypes.Structure):
        _fields_ = [("attributes", wintypes.DWORD),
                    ("created", wintypes.FILETIME), ("accessed", wintypes.FILETIME),
                    ("written", wintypes.FILETIME), ("volume", wintypes.DWORD),
                    ("size_high", wintypes.DWORD), ("size_low", wintypes.DWORD),
                    ("links", wintypes.DWORD), ("index_high", wintypes.DWORD),
                    ("index_low", wintypes.DWORD)]

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetFileInformationByHandle.argtypes = [wintypes.HANDLE, ctypes.POINTER(Information)]
    kernel.GetFileInformationByHandle.restype = wintypes.BOOL
    info = Information()
    if not kernel.GetFileInformationByHandle(handle, ctypes.byref(info)):
        raise ctypes.WinError(ctypes.get_last_error())
    return info


def _fd_stamp(fd):
    if os.name != "nt":
        return _stamp(os.fstat(fd))
    import msvcrt
    info = _windows_information(msvcrt.get_osfhandle(fd))
    # Use one native identity/time representation throughout; neither zero/dummy
    # CRT inode fields nor pathname-vs-handle timestamp precision are authority.
    return (info.volume, (info.index_high << 32) | info.index_low,
            (info.size_high << 32) | info.size_low,
            (info.written.dwHighDateTime << 32) | info.written.dwLowDateTime,
            (info.created.dwHighDateTime << 32) | info.created.dwLowDateTime)


def _open_windows_image(path):
    # CPython pathname stat and CRT fstat need not expose identical identity or
    # timestamp fields. Open the object atomically without following its final
    # reparse point instead. Denying write/delete sharing holds the admitted file
    # against modification/replacement for the lifetime of the returned fd.
    import ctypes
    from ctypes import wintypes
    import msvcrt

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                  ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD,
                                  wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.GetFileType.argtypes = [wintypes.HANDLE]
    kernel.GetFileType.restype = wintypes.DWORD
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL
    # GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING, OPEN_REPARSE_POINT.
    handle = kernel.CreateFileW(str(path), 0x80000000, 1, None, 3, 0x00200000, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        info = _windows_information(handle)
        require(kernel.GetFileType(handle) == 1 and not (info.attributes & (0x400 | 0x10)),
                "Image is not a regular file or is a reparse point")
        fd = msvcrt.open_osfhandle(handle, os.O_RDONLY | os.O_BINARY)
        handle = None  # CRT now owns the native handle, including on close.
        try:
            os.set_inheritable(fd, False)
            return fd
        except BaseException:
            os.close(fd)
            raise
    finally:
        if handle is not None:
            kernel.CloseHandle(handle)


def _open_image(path):
    if os.name == "nt":
        return _open_windows_image(path)
    # Native sessions provide a private parent. On Unix compare the opened
    # object with the no-follow pathname observation to reject replacement.
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


def _read_at(fd, count, offset):
    if hasattr(os, "pread"):
        return os.pread(fd, count, offset)
    # The Windows private adapter executes one synchronous command at a time and
    # owns these descriptors exclusively. No other thread shares their offsets.
    os.lseek(fd, offset, os.SEEK_SET)
    return os.read(fd, count)


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
            self._allowed = allowed_write_targets(self._release)
            require(isinstance(images, dict) and images and images.keys() <= self._allowed,
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
                self._sources[name]["stamp"] = _fd_stamp(fd)
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
        require(_fd_stamp(source["fd"]) == source["stamp"], f"Verified image changed: {name}")
        # Compare native descriptor metadata consistently on Windows. The
        # reopened name is independently checked against reparse points too.
        check = _open_image(source["path"])
        try:
            info = os.fstat(check)
            require(stat.S_ISREG(info.st_mode) and _fd_stamp(check) == source["stamp"],
                    f"Verified image path changed: {name}")
        finally:
            os.close(check)

    def _validate_source(self, name):
        self._unchanged(name)
        source = self._sources[name]
        full = hashlib.sha256()
        chunks = []
        total = source["stamp"][2]
        self._report("Verify image", name, 0, total)
        for offset in range(0, source["stamp"][2], CHUNK):
            count = min(CHUNK, source["stamp"][2] - offset)
            data = _read_at(source["fd"], count, offset)
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
        require(name in self._allowed and name in self._sources, "Prohibited or unmanifested write target")
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
                data = _read_at(image["fd"], count, offset)
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
