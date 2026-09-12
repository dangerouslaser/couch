#!/usr/bin/env python3
"""Experimental installer policy engine. Real USB writes are deliberately unavailable."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import time

MODEL = "sanytron-ha100"
IDENTITY_PARTITIONS = {"proinfo", "nvram", "nvdata", "protect1", "protect2"}
REQUIRED_IMAGES = {"recovery", "userdata", "boot"}
WRITE_ORDER = ("recovery", "userdata", "logo", "odmdtbo", "boot")
# A full vendor factory restore rewrites the whole OS (system/vendor) and its
# boot chain as well, unlike a Couch install/restore which only touches the five
# partitions above. This broader set is permitted ONLY for a manifest whose
# purpose is "factory-restore" (see allowed_write_targets). The bootloader
# (preloader) is deliberately never written — the factory preloader already on
# the device is preserved — and no calibration/identity partition is ever
# writable through either order. OS and data first, boot chain last, so an
# interrupted flash leaves the old boot in place (and the preserved preloader
# always keeps the device re-flashable over the download agent).
FACTORY_WRITE_ORDER = ("system", "vendor", "secro", "odmdtbo", "logo", "cache",
                       "userdata", "recovery", "lk2", "lk", "boot")
# Partitions that must never be written by any path: calibration/identity plus
# the bootloader and the invisible/reserved firmware regions.
NEVER_WRITE = IDENTITY_PARTITIONS | {"preloader", "seccfg", "para", "expdb", "frp",
                                     "metadata", "oemkeystore", "keystore", "pgpt",
                                     "sgpt", "flashinfo"}
assert set(WRITE_ORDER).isdisjoint(NEVER_WRITE), "Couch write order touches a protected partition"
assert set(FACTORY_WRITE_ORDER).isdisjoint(NEVER_WRITE), "Factory write order touches a protected partition"
CHUNK = 1024 * 1024
REPO = Path(__file__).resolve().parents[2]


def allowed_write_targets(release):
    """Writable partitions permitted for this manifest. The broad factory set is
    only unlocked by an explicit purpose; every other manifest keeps the narrow
    Couch install/restore set. Neither set can name a protected partition."""
    order = FACTORY_WRITE_ORDER if release.get("purpose") == "factory-restore" else WRITE_ORDER
    require(set(order).isdisjoint(NEVER_WRITE), "Write order names a protected partition")
    return set(order)


class InstallError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise InstallError(message)


def regular(path):
    path = Path(path)
    require(not path.is_symlink() and stat.S_ISREG(path.stat().st_mode),
            f"Expected a regular, non-symlink file: {path}")
    return path


def read_json(path):
    regular(path)
    require(path.stat().st_size <= CHUNK, "JSON metadata exceeds 1 MiB")
    return json.loads(path.read_text())


def digest(path):
    h = hashlib.sha256()
    with regular(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(CHUNK), b""):
            h.update(chunk)
    return h.hexdigest()


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def save_json(path, value):
    temporary = path.with_suffix(".tmp")
    # Exclusive creation catches stale/interfering files rather than following symlinks.
    with temporary.open("x", encoding="utf-8") as stream:
        os.chmod(temporary, 0o600)
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    sync_directory(path.parent)


def layout(value):
    require(isinstance(value, dict) and value, "Missing partition layout")
    regions = []
    for name, region in value.items():
        require(re.fullmatch(r"[a-z][a-z0-9_]*", name), "Invalid partition name")
        start, size = region["offset"], region["size"]
        require(type(start) is int and type(size) is int and start >= 512 and size > 0
                and start % 512 == 0 and size % 512 == 0,
                f"Invalid byte offset/size for {name}")
        regions.append((start, start + size, name))
    regions.sort()
    for previous, current in zip(regions, regions[1:]):
        require(previous[1] <= current[0], f"Overlapping partitions: {previous[2]}, {current[2]}")
    require(IDENTITY_PARTITIONS | REQUIRED_IMAGES <= value.keys(), "Missing required partitions")
    return value


def identity_record(path):
    value = read_json(path)
    # Vendor checklist says 16 characters but its published example has 15.
    # Preserve the displayed serial exactly; do not pad or infer an NVRAM offset.
    require(isinstance(value.get("device_id"), str)
            and re.fullmatch(r"[A-Za-z0-9]{8,64}", value["device_id"]), "Invalid recorded Device ID")
    for key in ("wifi_mac", "bluetooth_mac"):
        mac = value.get(key, "")
        require(isinstance(mac, str) and re.fullmatch(r"(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}", mac),
                f"Invalid {key}")
        require(mac.lower() not in ("00:00:00:00:00:00", "ff:ff:ff:ff:ff:ff"), f"Invalid {key}")
    return {key: value[key] for key in ("device_id", "wifi_mac", "bluetooth_mac")}


def load_release(path, *, verify_images=True):
    path = regular(path).resolve()
    value = read_json(path)
    require(value.get("schema") == 1 and value.get("model") == MODEL, "Unsupported release/model")
    layout(value["partitions"])
    images = value["images"]
    require(isinstance(images, dict) and REQUIRED_IMAGES <= images.keys(), "Release needs recovery, userdata, boot")
    require(images.keys() <= set(WRITE_ORDER), "Release contains a prohibited write target")
    for name, image in images.items():
        require(name in value["partitions"], f"Missing target layout: {name}")
        filename = image["file"]
        require(isinstance(filename, str) and Path(filename).name == filename and filename not in (".", ".."),
                "Image filenames must be basenames inside the release directory")
        require(isinstance(image["sha256"], str) and re.fullmatch(r"[a-f0-9]{64}", image["sha256"]),
                f"{name}: invalid image SHA-256 metadata")
        # Restore binds to this exact manifest and independently verifies every
        # original backup. Missing/corrupt installation payloads must not prevent
        # recovery; metadata-only mode still validates all paths and allowlists.
        if not verify_images:
            continue
        source = regular(path.parent / filename)
        require(source.stat().st_size == value["partitions"][name]["size"],
                f"{name}: expected a raw, full-partition image with exact declared size")
        require(digest(source) == image["sha256"], f"{name}: image SHA-256 mismatch")
        with source.open("rb") as stream:
            header = stream.read(8)
            require(header[:4] != b"\x3a\xff\x26\xed", "Android sparse images are unsupported")
            if name in ("boot", "recovery"):
                require(header == b"ANDROID!", f"{name}: missing Android boot-image magic")
            if name == "userdata":
                stream.seek(1080)
                require(stream.read(2) == b"\x53\xef", "userdata: missing ext4 superblock magic")
    return value, path.parent


class FileDevice:
    """Regular-file test transport only; never opens a real block device or USB handle."""
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.description = read_json(self.root / "device.json")
        require(self.description.get("simulation") is True, "File device must declare simulation: true")
        require(self.description.get("model") == MODEL, "Wrong device model")
        require(isinstance(self.description.get("storage_id"), str) and self.description["storage_id"],
                "Missing stable storage identity")
        layout(self.description["partitions"])
        for name, region in self.description["partitions"].items():
            require(self.partition(name).stat().st_size == region["size"], f"Wrong simulated size: {name}")

    def partition(self, name):
        require(name in self.description["partitions"], "Unknown partition")
        return regular(self.root / f"{name}.img")

    def hash(self, name):
        return digest(self.partition(name))

    def backup(self, name, destination):
        with self.partition(name).open("rb") as source, destination.open("xb") as output:
            os.chmod(destination, 0o600)
            shutil.copyfileobj(source, output, CHUNK)
            output.flush()
            os.fsync(output.fileno())
        sync_directory(destination.parent)

    def write(self, name, source):
        require(name in WRITE_ORDER, "Prohibited write target")
        require(regular(source).stat().st_size == self.partition(name).stat().st_size, "Write size mismatch")
        # r+b does not truncate: a failed write leaves a recoverable partial partition.
        with source.open("rb") as input_file, self.partition(name).open("r+b") as output:
            shutil.copyfileobj(input_file, output, CHUNK)
            output.flush()
            os.fsync(output.fileno())


def preflight(release, device):
    require(device.description["model"] == release["model"], "Device model mismatch")
    require(device.description["partitions"] == release["partitions"],
            "Actual partition layout differs from release; repartitioning is unsupported")


def install_transaction(release, bundle, device, backup_dir, identity, confirm, resume=False):
    """Back up every affected partition before any write; explicit resume retries interrupted writes."""
    preflight(release, device)
    require(confirm == device.description["storage_id"], "Target confirmation does not match storage identity")
    backup_dir = Path(backup_dir).absolute()
    require(not backup_dir.is_symlink(), "Backup directory must not be a symlink")
    require(not backup_dir.resolve().is_relative_to(REPO), "Keep per-device backups outside the repository")
    names = sorted(IDENTITY_PARTITIONS | release["images"].keys())
    journal_path = backup_dir / "journal.json"
    binding = {"release": fingerprint(release), "device": fingerprint(device.description),
               "identity": fingerprint(identity)}
    if resume:
        require(backup_dir.is_dir() and backup_dir.stat().st_mode & 0o077 == 0, "Backup directory must be private (0700)")
        journal = read_json(journal_path)
        require(journal.get("schema") == 1, "Unsupported journal schema")
        require("restore" not in journal, "Restore has started; installation cannot resume")
        require(journal.get("binding") == binding, "Journal belongs to a different release/device/identity")
        require(journal.get("backup_complete") is True, "Backup was interrupted; use a new backup directory")
        require(read_json(backup_dir / "identity.json") == identity, "Recorded identity changed")
        require(read_json(backup_dir / "device.json") == device.description, "Recorded device layout changed")
        writes = journal.get("writes")
        require(isinstance(writes, dict) and writes.keys() <= release["images"].keys()
                and all(state in ("writing", "verified") for state in writes.values()),
                "Invalid journal write states")
        for name in names:
            path = backup_dir / f"{name}.img"
            require(path.stat().st_size == release["partitions"][name]["size"]
                    and digest(path) == journal["backups"][name], f"Damaged backup: {name}")
        for name in IDENTITY_PARTITIONS:
            require(device.hash(name) == journal["backups"][name], f"Device identity changed: {name}")
    else:
        require(not backup_dir.exists(), "Backup directory already exists; choose a new path or --resume")
        backup_dir.mkdir(mode=0o700, parents=True)
        needed = sum(release["partitions"][name]["size"] for name in names) + 16 * CHUNK
        require(shutil.disk_usage(backup_dir).free >= needed, "Insufficient free space for complete original backups")
        journal = {"schema": 1, "binding": binding, "backup_complete": False, "backups": {}, "writes": {}}
        save_json(backup_dir / "identity.json", identity)
        save_json(backup_dir / "device.json", device.description)
        save_json(journal_path, journal)
        for name in names:
            print(f"Backing up and independently reading back {name}…", flush=True)
            path = backup_dir / f"{name}.img"
            device.backup(name, path)
            original_hash = digest(path)
            require(path.stat().st_size == release["partitions"][name]["size"]
                    and original_hash == device.hash(name), f"Backup readback mismatch: {name}")
            journal["backups"][name] = original_hash
            save_json(journal_path, journal)
        journal["backup_complete"] = True
        save_json(journal_path, journal)
    # Validate the entire remaining plan before changing any partition. In
    # particular, a later verified partition must not fail only after an
    # earlier interrupted write has already been retried.
    for name, image in release["images"].items():
        source = regular(bundle / image["file"])
        require(source.stat().st_size == release["partitions"][name]["size"]
                and digest(source) == image["sha256"], f"Release image changed after preflight: {name}")
        state = journal["writes"].get(name)
        if state == "verified":
            require(device.hash(name) == image["sha256"], f"Previously verified partition changed: {name}")
        elif state is None:
            require(device.hash(name) == journal["backups"][name], f"Unwritten partition changed: {name}")
    for name in WRITE_ORDER:
        if name not in release["images"]:
            continue
        expected = release["images"][name]["sha256"]
        if journal["writes"].get(name) == "verified":
            require(device.hash(name) == expected, f"Previously verified partition changed: {name}")
            continue
        source = regular(bundle / release["images"][name]["file"])
        require(digest(source) == expected, f"Release image changed after preflight: {name}")
        journal["writes"][name] = "writing"
        save_json(journal_path, journal)
        print(f"{'SIMULATION: ' if device.description.get('simulation') else ''}Writing and verifying {name}…", flush=True)
        device.write(name, source)
        require(device.hash(name) == expected, f"Write readback mismatch: {name}; keep journal and backups")
        journal["writes"][name] = "verified"
        save_json(journal_path, journal)
    for name in IDENTITY_PARTITIONS:
        require(device.hash(name) == journal["backups"][name], f"Identity changed during install: {name}")
    journal["complete"] = True
    save_json(journal_path, journal)
    print("Simulation verified. No physical device was flashed." if device.description.get("simulation")
          else "Partition writes and identity readback verified. Normal OS startup is not yet verified.")


def install_simulated(release, bundle, device, backup_dir, identity, confirm, resume=False):
    require(device.description.get("simulation") is True, "Simulation requires a file-backed device")
    return install_transaction(release, bundle, device, backup_dir, identity, confirm, resume)


def restore_transaction(release, device, backup_dir, identity, confirm, resume=False):
    """Restore originals on a file fixture; keep recovery until the last write."""
    preflight(release, device)
    require(confirm == device.description["storage_id"], "Target confirmation does not match storage identity")
    backup_dir = Path(backup_dir).absolute()
    require(not backup_dir.is_symlink() and not backup_dir.resolve().is_relative_to(REPO),
            "Use original backups outside the repository")
    require(backup_dir.is_dir() and backup_dir.stat().st_mode & 0o077 == 0,
            "Backup directory must be private (0700)")
    journal_path = backup_dir / "journal.json"
    journal = read_json(journal_path)
    binding = {"release": fingerprint(release), "device": fingerprint(device.description),
               "identity": fingerprint(identity)}
    require(journal.get("schema") == 1 and journal.get("binding") == binding,
            "Journal belongs to a different release/device/identity")
    require(journal.get("backup_complete") is True, "Original backup is incomplete")
    require(read_json(backup_dir / "identity.json") == identity and
            read_json(backup_dir / "device.json") == device.description, "Recorded identity/layout changed")
    writes = journal.get("writes")
    require(isinstance(writes, dict) and writes.keys() <= release["images"].keys() and
            all(state in ("writing", "verified") for state in writes.values()), "Invalid install write states")
    names = IDENTITY_PARTITIONS | release["images"].keys()
    require(isinstance(journal.get("backups"), dict) and journal["backups"].keys() == names,
            "Original backup inventory differs")
    for name in names:
        path = regular(backup_dir / f"{name}.img")
        require(path.stat().st_size == release["partitions"][name]["size"] and
                digest(path) == journal["backups"][name], f"Damaged backup: {name}")
    for name in IDENTITY_PARTITIONS:
        require(device.hash(name) == journal["backups"][name], f"Device identity changed: {name}")
    if resume:
        restored = journal.get("restore")
        require(isinstance(restored, dict), "No restore to resume")
    else:
        require("restore" not in journal, "Restore already started; use --resume")
        restored = {}
    require(restored.keys() <= release["images"].keys() and
            all(state in ("writing", "verified") for state in restored.values()), "Invalid restore states")
    # Validate every untouched/verified target before the first write. Only a
    # journaled interrupted write may contain an unknown partial image.
    for name, image in release["images"].items():
        state = restored.get(name)
        current = device.hash(name)
        if state == "verified":
            require(current == journal["backups"][name], f"Previously restored partition changed: {name}")
        elif state != "writing" and writes.get(name) != "writing":
            expected = image["sha256"] if writes.get(name) == "verified" else journal["backups"][name]
            require(current == expected, f"Partition changed before restore: {name}")
    journal["restore"] = restored
    journal["restore_complete"] = False
    save_json(journal_path, journal)
    for name in ("userdata", "logo", "odmdtbo", "boot", "recovery"):
        if name not in release["images"] or restored.get(name) == "verified":
            continue
        source = regular(backup_dir / f"{name}.img")
        expected = journal["backups"][name]
        require(digest(source) == expected, f"Backup changed before restore: {name}")
        restored[name] = "writing"
        save_json(journal_path, journal)
        print(f"{'SIMULATION: ' if device.description.get('simulation') else ''}Restoring and verifying original {name}…", flush=True)
        device.write(name, source)
        require(device.hash(name) == expected, f"Restore readback mismatch: {name}; retain originals and journal")
        restored[name] = "verified"
        save_json(journal_path, journal)
    for name in IDENTITY_PARTITIONS:
        require(device.hash(name) == journal["backups"][name], f"Identity changed during restore: {name}")
    journal["restore_complete"] = True
    save_json(journal_path, journal)
    print("Restore simulation verified. No physical device was flashed." if device.description.get("simulation")
          else "Original partitions and identity readback verified. Startup still requires verification.")


def restore_simulated(release, device, backup_dir, identity, confirm, resume=False):
    require(isinstance(device, FileDevice), "Restore supports file simulation only")
    return restore_transaction(release, device, backup_dir, identity, confirm, resume)


def watch_usb(timeout):
    # These are generic MediaTek IDs, not proof that a candidate is an HA100.
    try:
        import usb.core
    except ImportError as error:
        raise InstallError("USB discovery needs PyUSB and a libusb backend; see docs/installer.md") from error
    require(0 < timeout <= 3600, "Timeout must be between 1 and 3600 seconds")
    print("Watching USB descriptors only. Connect the remote and reboot; no handshake or writes will be sent.", flush=True)
    deadline = time.monotonic() + timeout
    known = set()
    while time.monotonic() < deadline:
        devices = list(usb.core.find(find_all=True, idVendor=0x0e8d))
        for dev in devices:
            if dev.idProduct not in (0x0003, 0x6000, 0x2000, 0x2001, 0x20ff, 0x3000):
                continue
            key = (dev.bus, dev.address, dev.idProduct)
            if key not in known:
                known.add(key)
                print(f"MediaTek candidate: bus={dev.bus} address={dev.address} "
                      f"vid=0e8d pid={dev.idProduct:04x}; model and identity UNVERIFIED", flush=True)
        time.sleep(0.05)
    require(known, "No MediaTek download-mode candidate observed before timeout")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    watch = commands.add_parser("watch-usb", help="Observe generic MTK USB enumeration; does not capture the boot session")
    watch.add_argument("--timeout", type=float, default=60)
    commands.add_parser("install", help="Unavailable until the HA100 USB transport is hardware-validated")
    for name in ("plan", "simulate", "simulate-restore"):
        command = commands.add_parser(name)
        command.add_argument("--manifest", type=Path, required=True)
        command.add_argument("--device-dir", type=Path, required=True, help="File-backed simulated device")
        if name in ("simulate", "simulate-restore"):
            command.add_argument("--backup-dir", type=Path, required=True)
            command.add_argument("--identity", type=Path, required=True)
            command.add_argument("--confirm-device", required=True)
            command.add_argument("--resume", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "install":
            raise InstallError("USB installation is not enabled: pinned MTK transport, loader and HA100 validation are still required. Use plan/simulate.")
        if args.command == "watch-usb":
            watch_usb(args.timeout)
            return 0
        release, bundle = load_release(args.manifest)
        device = FileDevice(args.device_dir)
        preflight(release, device)
        if args.command == "plan":
            print("SIMULATED DEVICE — dry run; no files or partitions will be written")
            print("Backup: " + ", ".join(sorted(IDENTITY_PARTITIONS | release["images"].keys())))
            print("Write and verify: " + " → ".join(name for name in WRITE_ORDER if name in release["images"]))
            print("Preserve every other partition; no format, repartition, bootloader writes or automatic reboot.")
        elif args.command == "simulate-restore":
            restore_simulated(release, device, args.backup_dir, identity_record(args.identity),
                              args.confirm_device, args.resume)
        else:
            install_simulated(release, bundle, device, args.backup_dir, identity_record(args.identity),
                              args.confirm_device, args.resume)
        return 0
    except (InstallError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"Stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
