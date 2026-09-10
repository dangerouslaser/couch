#!/usr/bin/env python3
"""Explicit experimental USB identity capture. Never resets or flashes the remote."""
import argparse
import hashlib
from pathlib import Path
import re
import sys
import time

from couch_install import IDENTITY_PARTITIONS, InstallError, REPO, layout, read_json, require, save_json
from mtk_readonly import REVIEWED_REVISION
from mtk_session import DOWNLOAD_PIDS, loader_bytes, read_session, source_pin
from mtk_usb import ExactUsbBackend


def baseline_record(path):
    if path is None:
        return None
    value = read_json(path)
    require(value.get("schema") == 1, "Unsupported runtime baseline schema")
    capacity = value.get("capacity")
    require(type(capacity) is int and capacity > 0 and capacity % 512 == 0, "Invalid runtime capacity")
    layout(value["partitions"])
    require(all(region["offset"] + region["size"] <= capacity for region in value["partitions"].values()),
            "Runtime partition exceeds capacity")
    if "cid" in value:
        require(isinstance(value["cid"], str) and re.fullmatch(r"[0-9a-fA-F]{32}", value["cid"]), "Invalid runtime CID")
        require(value["cid"].lower() not in ("0" * 32, "f" * 32), "Invalid runtime CID")
    hashes = value.get("identity_sha256", {})
    require(isinstance(hashes, dict) and (not hashes or set(hashes) == IDENTITY_PARTITIONS),
            "Runtime identity hashes must cover all five calibration partitions")
    require(all(isinstance(item, str) and re.fullmatch(r"[0-9a-f]{64}", item) for item in hashes.values()),
            "Invalid runtime identity hash")
    return value


def compare_baseline(reader, baseline, *, progress=None):
    if baseline is None:
        return
    observed = reader.description
    require(observed["capacity"] == baseline["capacity"], "Observed storage capacity differs from runtime baseline")
    require(observed["partitions"] == baseline["partitions"], "Observed partition layout differs from runtime baseline")
    if "cid" in baseline:
        require(observed.get("cid_encoding") == "mt6580-legacy-le32-registers"
                and observed.get("runtime_cid_sha256") == hashlib.sha256(bytes.fromhex(baseline["cid"])).hexdigest(),
                "Observed eMMC CID differs from runtime baseline")
    for name, checksum in baseline.get("identity_sha256", {}).items():
        if progress:
            progress(f"Verifying runtime baseline: {name}")
        require(reader.hash(name) == checksum, f"Runtime identity baseline differs: {name}")


def wait_preloader(enumerate_devices, bus, ports, timeout, *, clock=time.monotonic, sleep=time.sleep):
    deadline = clock() + timeout
    while clock() < deadline:
        observed = enumerate_devices()
        candidates = [item for item in observed if item.vid == 0x0e8d and item.pid in DOWNLOAD_PIDS]
        require(len(candidates) <= 1, "Multiple MediaTek download devices present; refusing selection")
        if candidates:
            candidate = candidates[0]
            require(candidate.bus == bus and candidate.ports == ports, "Download device appeared on a different USB port")
            require(candidate.pid == 0x2000, "Only MTK preloader PID 2000 is enabled; BootROM is unsupported")
            return candidate
        sleep(0.05)
    raise InstallError("No supported preloader appeared on the selected USB port before timeout")


def capture(args, *, enumerate_devices=None, session=read_session, allow_identity_refresh=False):
    ports = tuple(int(part) for part in args.ports.split("."))
    require(args.bus > 0 and ports and all(part > 0 for part in ports), "Invalid USB bus/port path")
    require(0 < args.timeout <= 600, "Timeout must be between 1 and 600 seconds")
    destination = args.backup_dir.absolute()
    require(not destination.is_symlink() and not destination.exists(), "Use a new identity backup destination")
    require(not destination.resolve().is_relative_to(REPO), "Keep identity backups outside the repository")
    baseline = baseline_record(args.baseline)
    if allow_identity_refresh:
        require(baseline is not None and "cid" in baseline,
                "Identity refresh requires an explicitly trusted CID and complete layout binding")
    boot_after_capture = getattr(args, "boot_after_capture", False)
    if boot_after_capture:
        require(baseline is not None and "cid" in baseline
                and (bool(baseline.get("identity_sha256")) or allow_identity_refresh),
                "Boot-after-capture requires a complete runtime identity baseline")
    # Complete all file checks before even importing PyUSB for discovery.
    source_pin(args.checkout)
    loader_bytes(args.loader, args.loader_sha256)
    loader_bytes(args.preloader, args.preloader_sha256)
    if args.check_only:
        print("Source, loader, board-data hashes and baseline syntax verified. No USB operation performed.")
        return
    def candidate_provider(enumerate_backend):
        print(f"Prepared. Waiting on USB bus {args.bus}, port {args.ports}, preloader PID 2000. No USB reset will be sent.", flush=True)
        candidate = wait_preloader(enumerate_devices or enumerate_backend, args.bus, ports, args.timeout)
        print("Selected preloader found. Starting a read-only DA session…", flush=True)
        return candidate
    factory = lambda checkout: ExactUsbBackend(checkout, preloader=args.preloader,
                                               preloader_sha256=args.preloader_sha256)
    with session(args.checkout, args.loader, args.loader_sha256, args.lock_dir, None, factory,
                 candidate_provider=candidate_provider, boot_after_capture=boot_after_capture) as reader:
        # Hardware layout/CID only exist after DA startup. Baseline syntax was
        # checked before USB; actual metadata and bytes are compared here.
        progress = lambda message: print(message, flush=True)
        progress("DA startup and both GPT copies verified. Checking runtime baseline before creating backup files.")
        compare_baseline(reader, baseline, progress=progress)
        report = reader.backup_identity(destination, reader.description["storage_id"], progress=progress)
        if baseline is not None:
            for name, checksum in baseline.get("identity_sha256", {}).items():
                if report["backups"][name] != checksum:
                    report["complete"] = False
                    report["runtime_baseline_verified"] = False
                    save_json(destination / "readback.json", report)
                    raise InstallError(f"Captured identity differs from runtime baseline: {name}")
        report["runtime_baseline_verified"] = baseline is not None
        report["runtime_cid_verified"] = baseline is not None and "cid" in baseline
        report["runtime_identity_verified"] = baseline is not None and bool(baseline.get("identity_sha256"))
        report["loader_sha256"] = args.loader_sha256
        report["board_data_sha256"] = args.preloader_sha256
        report["revision"] = REVIEWED_REVISION
        report["boot_requested"] = boot_after_capture
        report["boot_acknowledged"] = False
        report["normal_os_verified"] = False
        report["usb_cleanup_verified"] = False
        save_json(destination / "readback.json", report)
        progress("Identity backup and independent readback verified. " +
                 ("Requesting normal boot, then releasing USB interfaces." if boot_after_capture else
                  "Releasing USB interfaces; Couch has not been rebooted."))
    report["boot_acknowledged"] = boot_after_capture
    report["usb_cleanup_verified"] = True
    save_json(destination / "readback.json", report)
    print("Read-only capture verified and session closed. No partition writes were sent.")
    if boot_after_capture:
        print("DA exit acknowledged. On the tested HA100 this powers off; hold Power to start Couch.")
        print("Couch startup is not verified by this capture command.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--loader", type=Path, required=True)
    parser.add_argument("--loader-sha256", required=True)
    parser.add_argument("--preloader", type=Path, required=True, help="Same-device original board data; never flashed")
    parser.add_argument("--preloader-sha256", required=True)
    parser.add_argument("--backup-dir", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--bus", type=int, required=True)
    parser.add_argument("--ports", required=True, help="Physical USB port path, e.g. 1 or 1.3")
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--lock-dir", type=Path, default=Path.home() / ".local/state/couch-installer/locks")
    parser.add_argument("--boot-after-capture", action="store_true",
                        help="Experimental: request DA exit after verified capture; tested HA100 needs manual Power; requires full baseline")
    parser.add_argument("--check-only", action="store_true", help="Validate local inputs without importing USB or waiting")
    args = parser.parse_args()
    try:
        capture(args)
        return 0
    except (InstallError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"Capture stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
