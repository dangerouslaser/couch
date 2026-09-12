#!/usr/bin/env python3
"""Vendor factory-firmware restore over the download agent.

Unlike the Couch install/restore path (which only writes recovery, userdata,
logo, odmdtbo and boot) and unlike stock_baseline_restore (which writes just
userdata/odmdtbo/boot and assumes the stock system is intact), this restores a
complete vendor factory image: system and vendor included, plus the boot chain
(lk/lk2/boot/recovery), logo, secro and a fresh empty cache/userdata. It never
writes the preloader — the factory preloader already on the device is preserved
— and never writes any calibration/identity partition; those are verified
unchanged against the runtime baseline before and after. Every partition is read
back and hashed independently after writing. One explicit attempt, no resume.

The manifest must be a reviewed factory-restore bundle built from the pinned
vendor archive (see tools/release/firmware_restore.py and
ha100_factory_firmware.json): sparse images already expanded, every image a raw
full-partition image of the exact device size with a recorded sha256.
"""
import argparse
import hashlib
from pathlib import Path
import re
import struct
import sys

from capture_readonly import baseline_record, compare_baseline, wait_preloader
from couch_install import (FACTORY_WRITE_ORDER, MODEL, REPO, InstallError, NEVER_WRITE,
                           digest, fingerprint, layout, read_json, regular, require,
                           save_json, sync_directory)
from mtk_readonly import ConnectedMtkReader
from mtk_session import connected_session, loader_bytes, source_pin
from mtk_usb import ExactUsbBackend
from mtk_writer import ConnectedMtkWriter

SPARSE_MAGIC = b"\x3a\xff\x26\xed"
F2FS_SUPERBLOCK = struct.pack("<I", 0xF2F52010)
EXT4_MAGIC = b"\x53\xef"  # little-endian 0xEF53 at offset 0x438


def _write_set(images):
    """Manifest image names ordered for writing; boot chain last."""
    return [name for name in FACTORY_WRITE_ORDER if name in images]


def load_factory_manifest(path):
    path = regular(path).resolve()
    release = read_json(path)
    require(isinstance(release, dict), "Expected a factory manifest object")
    require(release.get("schema") == 1 and release.get("model") == MODEL
            and release.get("private_candidate") is True and release.get("installable") is False
            and release.get("purpose") == "factory-restore",
            "Expected a reviewed private factory-restore manifest")
    layout(release["partitions"])
    images = release.get("images")
    require(isinstance(images, dict) and images, "Factory restore requires an image set")
    require(images.keys() <= set(FACTORY_WRITE_ORDER), "Factory manifest names a prohibited write target")
    require(set(images).isdisjoint(NEVER_WRITE), "Factory manifest names a protected partition")
    # A coherent bootable factory state needs at least the OS and its boot chain.
    require({"system", "vendor", "boot"} <= images.keys(), "Factory restore needs system, vendor and boot")
    for name in _write_set(images):
        image = images[name]
        require(isinstance(image, dict), "Invalid factory image entry")
        filename = image.get("file")
        require(isinstance(filename, str) and Path(filename).name == filename and filename not in ("", ".", ".."),
                "Factory image names must be local basenames")
        require(name in release["partitions"], "Factory restore target missing from layout")
        source = regular(path.parent / filename)
        require(source.stat().st_size == release["partitions"][name]["size"],
                f"{name}: exact full-partition image required")
        require(isinstance(image.get("sha256"), str) and re.fullmatch(r"[0-9a-f]{64}", image["sha256"]),
                "Invalid factory image hash")
        with source.open("rb") as stream:
            head = stream.read(8)
            require(head[:4] != SPARSE_MAGIC, "Expand sparse factory images before restoration")
            if name == "boot":
                require(head == b"ANDROID!", "Factory boot image lacks Android magic")
            if name in ("system", "vendor"):
                stream.seek(0x438)
                require(stream.read(2) == EXT4_MAGIC, f"Factory {name} is not an ext4 image")
            if name == "userdata":
                for offset in (1024, 5120):
                    stream.seek(offset)
                    require(stream.read(4) == F2FS_SUPERBLOCK, "Factory userdata requires both F2FS superblocks")
        require(digest(source) == image["sha256"], f"Factory image SHA-256 mismatch: {name}")
    return release, path.parent


def validate(args):
    require(args.allow_private_flash, "Factory restore requires --allow-private-flash")
    require(args.confirm_factory_reset, "Factory restore wipes data; pass --confirm-factory-reset")
    release, bundle = load_factory_manifest(args.manifest)
    baseline = baseline_record(args.baseline)
    require(baseline is not None and "cid" in baseline and baseline.get("identity_sha256"),
            "A complete independently verified runtime baseline is required")
    require(release["partitions"] == baseline["partitions"], "Factory layout differs from runtime baseline")
    require(args.confirm_cid_sha256 == hashlib.sha256(bytes.fromhex(baseline["cid"])).hexdigest(),
            "Explicit CID confirmation mismatch")
    ports = tuple(int(part) for part in args.ports.split("."))
    require(args.bus > 0 and ports and all(part > 0 for part in ports), "Invalid physical USB port")
    require(0 < args.timeout <= 600, "Timeout must be between 1 and 600 seconds")
    journal_dir = args.journal_dir.absolute()
    require(not journal_dir.is_symlink() and not journal_dir.exists(),
            "Use a new factory-restore journal directory; no automatic resume")
    require(not journal_dir.resolve().is_relative_to(REPO), "Keep factory restore journals outside the repository")
    source_pin(args.checkout)
    loader_bytes(args.loader, args.loader_sha256)
    loader_bytes(args.preloader, args.preloader_sha256)
    return release, bundle, baseline, ports, journal_dir


def restore_factory(release, bundle, baseline, device, journal_dir, boot_requested):
    """One explicit attempt. Calibration is preserved and verified, never written."""
    require(device.description["model"] == MODEL and device.description["partitions"] == release["partitions"],
            "Writer differs from bound factory restore target")
    order = _write_set(release["images"])
    for name in order:
        source = regular(bundle / release["images"][name]["file"])
        require(source.stat().st_size == release["partitions"][name]["size"]
                and digest(source) == release["images"][name]["sha256"], f"Factory image changed: {name}")
    journal_dir.mkdir(mode=0o700, parents=True)
    sync_directory(journal_dir.parent)
    path = journal_dir / "journal.json"
    record = {"schema": 1, "purpose": "vendor-factory-restore", "complete": False,
              "factory_reset": True, "preserved_preloader": True,
              "source": release.get("source"),
              "binding": {"release": fingerprint(release), "device": fingerprint(device.description),
                          "baseline": fingerprint(baseline)}, "order": order, "writes": {},
              "boot_requested": boot_requested, "boot_acknowledged": False,
              "usb_cleanup_verified": False, "identity_verified_after": False}
    save_json(path, record)
    try:
        # Calibration/identity is untouched by this flow; confirm it matches the
        # baseline before writing so a mismatched device is rejected up front.
        compare_baseline(device, baseline, progress=lambda message: print(message, flush=True))
        record["identity_verified_before"] = True
        save_json(path, record)
        for name in order:
            record["writes"][name] = "writing"
            save_json(path, record)
            print(f"Restoring factory {name}…", flush=True)
            device.write(name, bundle / release["images"][name]["file"])
            require(device.hash(name) == release["images"][name]["sha256"],
                    f"Factory {name} independent readback mismatch; do not retry this session")
            record["writes"][name] = "verified"
            save_json(path, record)
        # Identity must still be intact after writing the OS and boot chain.
        compare_baseline(device, baseline, progress=lambda message: print(message, flush=True))
        record["write_readback_complete"] = True
        record["identity_verified_after"] = True
        save_json(path, record)
        return record
    except BaseException as error:
        record["stopped"] = type(error).__name__
        save_json(path, record)
        raise


def run(args, *, session=connected_session, reader_factory=ConnectedMtkReader, writer_factory=ConnectedMtkWriter):
    release, bundle, baseline, ports, journal_dir = validate(args)
    if args.check_only:
        print("Factory images (raw, full-partition, hashed), layout, CID confirmation and pinned inputs "
              "verified. No USB operation performed.")
        return

    def choose(enumerate_devices):
        print(f"Ready on USB bus {args.bus}, port {args.ports}. Waiting for preloader; no reset will be sent.",
              flush=True)
        return wait_preloader(enumerate_devices, args.bus, ports, args.timeout)

    def device_factory(mtk, revision):
        reader = reader_factory(mtk, revision)
        compare_baseline(reader, baseline, progress=lambda message: print(message, flush=True))
        binding = {**reader.description, "model": MODEL, "model_verified": True,
                   "identity_sha256": fingerprint(baseline["identity_sha256"])}
        return writer_factory(mtk, revision, release=release, bundle=bundle, binding=binding)

    factory = lambda checkout: ExactUsbBackend(checkout, preloader=args.preloader,
                                               preloader_sha256=args.preloader_sha256)
    with session(args.checkout, args.loader, args.loader_sha256, args.lock_dir, None, factory,
                 candidate_provider=choose, device_factory=device_factory,
                 boot_after_capture=args.boot_after_restore) as device:
        with device:
            record = restore_factory(release, bundle, baseline, device, journal_dir, args.boot_after_restore)
    record.update(complete=True, usb_cleanup_verified=True, boot_acknowledged=args.boot_after_restore)
    save_json(journal_dir / "journal.json", record)
    print("Factory OS, boot chain and retained identity verified. Normal startup still requires verification.")
    if args.boot_after_restore:
        print("DA exit acknowledged; hold Power if the HA100 remains off.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-private-flash", action="store_true")
    parser.add_argument("--confirm-factory-reset", action="store_true",
                        help="acknowledge that cache and userdata (/data) are wiped")
    for name in ("manifest", "baseline", "checkout", "loader", "preloader", "journal-dir"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    for name in ("loader-sha256", "preloader-sha256", "confirm-cid-sha256", "ports"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--bus", type=int, required=True)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--lock-dir", type=Path, default=Path.home() / ".local/state/couch-installer/locks")
    parser.add_argument("--boot-after-restore", "--boot-after-install", dest="boot_after_restore", action="store_true")
    parser.add_argument("--check-only", action="store_true")
    try:
        run(parser.parse_args())
    except (InstallError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"Factory restore stopped: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("Factory restore interrupted. Keep the journal; discard this DA session. No automatic retry or reset.",
              file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
