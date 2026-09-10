#!/usr/bin/env python3
"""Developer-only stock restoration that explicitly discards a disposable test OS.

This is not the public installer/restore path: it deliberately does not back up
Couch userdata. Only userdata, odmdtbo and boot may change; recovery remains intact.
"""
import argparse
import hashlib
from pathlib import Path
import re
import struct
import sys

from capture_readonly import baseline_record, compare_baseline, wait_preloader
from couch_install import (MODEL, REPO, InstallError, digest, fingerprint, layout,
                           read_json, regular, require, save_json, sync_directory)
from mtk_readonly import ConnectedMtkReader
from mtk_session import connected_session, loader_bytes, source_pin
from mtk_usb import ExactUsbBackend
from mtk_writer import ConnectedMtkWriter

TARGETS = ("userdata", "odmdtbo", "boot")


def load_stock_manifest(path):
    path = regular(path).resolve()
    release = read_json(path)
    require(isinstance(release, dict), "Expected a stock manifest object")
    require(release.get("schema") == 1 and release.get("model") == MODEL
            and release.get("private_candidate") is True and release.get("installable") is False
            and release.get("purpose") == "stock-baseline", "Expected a reviewed private stock-baseline manifest")
    layout(release["partitions"])
    images = release.get("images")
    require(isinstance(images, dict) and set(images) == set(TARGETS), "Stock restore requires exactly userdata, odmdtbo and boot")
    for name in TARGETS:
        image = images[name]
        require(isinstance(image, dict), "Invalid stock image entry")
        filename = image.get("file")
        require(isinstance(filename, str) and Path(filename).name == filename and filename not in ("", ".", ".."),
                "Stock image names must be local basenames")
        require(name in release["partitions"], "Stock restore target missing from layout")
        source = regular(path.parent / filename)
        require(source.stat().st_size == release["partitions"][name]["size"], f"{name}: exact full-partition image required")
        require(isinstance(image.get("sha256"), str) and re.fullmatch(r"[0-9a-f]{64}", image["sha256"]),
                "Invalid stock image hash")
        with source.open("rb") as stream:
            magic = stream.read(8)
            require(magic[:4] != b"\x3a\xff\x26\xed", "Expand sparse stock images before restoration")
            if name == "boot":
                require(magic == b"ANDROID!", "Stock boot image lacks Android magic")
            if name == "userdata":
                for offset in (1024, 5120):
                    stream.seek(offset)
                    require(stream.read(4) == struct.pack("<I", 0xf2f52010), "Stock userdata requires both F2FS superblocks")
        require(digest(source) == image["sha256"], f"Stock image SHA-256 mismatch: {name}")
    return release, path.parent


def validate(args):
    require(args.discard_test_installation, "This destructive stock trial requires --discard-test-installation")
    require(args.allow_private_flash, "Stock trial requires --allow-private-flash")
    release, bundle = load_stock_manifest(args.manifest)
    baseline = baseline_record(args.baseline)
    require(baseline is not None and "cid" in baseline and baseline.get("identity_sha256"),
            "A complete independently verified runtime baseline is required")
    require(release["partitions"] == baseline["partitions"], "Stock layout differs from runtime baseline")
    require(args.confirm_cid_sha256 == hashlib.sha256(bytes.fromhex(baseline["cid"])).hexdigest(),
            "Explicit CID confirmation mismatch")
    ports = tuple(int(part) for part in args.ports.split("."))
    require(args.bus > 0 and ports and all(part > 0 for part in ports), "Invalid physical USB port")
    require(0 < args.timeout <= 600, "Timeout must be between 1 and 600 seconds")
    journal_dir = args.journal_dir.absolute()
    require(not journal_dir.is_symlink() and not journal_dir.exists(), "Use a new stock-restore journal directory; no automatic resume")
    require(not journal_dir.resolve().is_relative_to(REPO), "Keep stock restore journals outside the repository")
    source_pin(args.checkout)
    loader_bytes(args.loader, args.loader_sha256)
    loader_bytes(args.preloader, args.preloader_sha256)
    return release, bundle, baseline, ports, journal_dir


def restore_stock(release, bundle, baseline, device, journal_dir, boot_requested):
    """One explicit attempt. Never back up or claim to preserve disposable userdata."""
    require(device.description["model"] == MODEL and device.description["partitions"] == release["partitions"],
            "Writer differs from bound stock restore target")
    # Validate every source again immediately before creating the write journal.
    for name in TARGETS:
        source = regular(bundle / release["images"][name]["file"])
        require(source.stat().st_size == release["partitions"][name]["size"]
                and digest(source) == release["images"][name]["sha256"], f"Stock image changed: {name}")
    journal_dir.mkdir(mode=0o700, parents=True)
    sync_directory(journal_dir.parent)
    path = journal_dir / "journal.json"
    record = {"schema": 1, "purpose": "discard-test-installation-stock-baseline", "complete": False,
              "discard_test_installation": True, "userdata_backup_taken": False,
              "binding": {"release": fingerprint(release), "device": fingerprint(device.description),
                          "baseline": fingerprint(baseline)}, "writes": {},
              "boot_requested": boot_requested, "boot_acknowledged": False,
              "usb_cleanup_verified": False, "normal_os_verified": False}
    save_json(path, record)
    try:
        recovery = device.hash("recovery")
        record["retained_recovery_sha256"] = recovery
        save_json(path, record)
        for name in TARGETS:
            record["writes"][name] = "writing"
            save_json(path, record)
            print(f"Restoring stock {name}; disposable Couch userdata is not backed up…", flush=True)
            device.write(name, bundle / release["images"][name]["file"])
            require(device.hash(name) == release["images"][name]["sha256"], f"Stock {name} independent readback mismatch; do not retry this session")
            record["writes"][name] = "verified"
            save_json(path, record)
        require(device.hash("recovery") == recovery, "Retained recovery changed during stock restoration")
        compare_baseline(device, baseline, progress=lambda message: print(message, flush=True))
        record["write_readback_complete"] = True
        record["identity_verified_after"] = True
        record["recovery_verified_after"] = True
        save_json(path, record)
        return record
    except BaseException as error:
        record["stopped"] = type(error).__name__
        save_json(path, record)
        raise


def run(args, *, session=connected_session, reader_factory=ConnectedMtkReader, writer_factory=ConnectedMtkWriter):
    release, bundle, baseline, ports, journal_dir = validate(args)
    if args.check_only:
        print("Stock F2FS/raw images, full hashes, layout, CID confirmation and pinned inputs verified. No USB operation performed.")
        return

    def choose(enumerate_devices):
        print(f"Ready on USB bus {args.bus}, port {args.ports}. Waiting for preloader; no reset will be sent.", flush=True)
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
            record = restore_stock(release, bundle, baseline, device, journal_dir, args.boot_after_restore)
    record.update(complete=True, usb_cleanup_verified=True, boot_acknowledged=args.boot_after_restore)
    save_json(journal_dir / "journal.json", record)
    print("Stock target partitions and retained identity/recovery verified. Normal startup still requires verification.")
    if args.boot_after_restore:
        print("DA exit acknowledged; hold Power if the HA100 remains off.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--discard-test-installation", action="store_true")
    parser.add_argument("--allow-private-flash", action="store_true")
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
        print(f"Stock restore stopped: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("Stock restore interrupted. Keep the journal; discard this DA session. No automatic retry or reset.", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
