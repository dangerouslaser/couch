#!/usr/bin/env python3
"""Developer-only HA100 installation trial. Not a public installer or release."""
import argparse
import hashlib
from pathlib import Path
import sys

from capture_readonly import baseline_record, compare_baseline, wait_preloader
from couch_install import (MODEL, InstallError, fingerprint, install_transaction, load_release,
                           read_json, restore_transaction, require)
from mtk_readonly import ConnectedMtkReader
from mtk_session import connected_session, loader_bytes, source_pin
from mtk_usb import ExactUsbBackend


def validate_inputs(args):
    require(args.allow_private_flash, "Physical trial requires --allow-private-flash")
    release, bundle = load_release(args.manifest, verify_images=not getattr(args, "restore", False))
    require(release.get("private_candidate") is True and release.get("installable") is False,
            "Use a reviewed private candidate, never a simulation or public release")
    baseline = baseline_record(args.baseline)
    require(baseline is not None and "cid" in baseline and baseline.get("identity_sha256"),
            "A complete independently captured runtime baseline is required")
    require(release["partitions"] == baseline["partitions"], "Candidate differs from known device layout")
    cid_hash = hashlib.sha256(bytes.fromhex(baseline["cid"])).hexdigest()
    require(args.confirm_cid_sha256 == cid_hash, "Explicit target CID confirmation mismatch")
    ports = tuple(int(part) for part in args.ports.split("."))
    require(args.bus > 0 and ports and all(part > 0 for part in ports), "Invalid physical USB port")
    require(0 < args.timeout <= 600, "Timeout must be between 1 and 600 seconds")
    source_pin(args.checkout)
    loader_bytes(args.loader, args.loader_sha256)
    loader_bytes(args.preloader, args.preloader_sha256)
    return release, bundle, baseline, ports


def run(args, *, session=connected_session):
    release, bundle, baseline, ports = validate_inputs(args)
    if args.check_only:
        print("Private manifest metadata, layout, target and loader inputs verified. Original backups will be checked before restore; no USB operation performed." if getattr(args, "restore", False) else "Private image hashes, layout, target and loader inputs verified. No USB operation performed.")
        return
    from mtk_writer import ConnectedMtkWriter

    def choose(enumerate_devices):
        print(f"Ready on USB bus {args.bus}, port {args.ports}. Reboot the connected remote now.", flush=True)
        return wait_preloader(enumerate_devices, args.bus, ports, args.timeout)

    def device_factory(mtk, revision):
        reader = ConnectedMtkReader(mtk, revision)
        print("Checking both GPT copies, storage identity and calibration against the runtime baseline…", flush=True)
        compare_baseline(reader, baseline, progress=lambda text: print(text, flush=True))
        binding = {**reader.description, "model": MODEL, "model_verified": True,
                   "identity_sha256": fingerprint(baseline["identity_sha256"])}
        write_release, write_bundle = release, bundle
        if args.restore:
            journal = read_json(args.backup_dir / "journal.json")
            require(journal.get("backup_complete") is True, "Original backup is incomplete")
            write_release = {**release, "images": {
                name: {"file": f"{name}.img", "sha256": journal["backups"][name]}
                for name in release["images"]}}
            write_bundle = args.backup_dir
        return ConnectedMtkWriter(mtk, revision, release=write_release, bundle=write_bundle, binding=binding)

    factory = lambda checkout: ExactUsbBackend(checkout, preloader=args.preloader,
                                               preloader_sha256=args.preloader_sha256)
    with session(args.checkout, args.loader, args.loader_sha256, args.lock_dir, None, factory,
                 candidate_provider=choose, device_factory=device_factory,
                 boot_after_capture=args.boot_after_install) as device:
        # Binary preservation is verified. Do not invent decoded vendor serial/MACs.
        identity = {"kind": "verified-runtime-binary-baseline", "cid_sha256": args.confirm_cid_sha256,
                    "identity_sha256": baseline["identity_sha256"]}
        with device:
            if args.restore:
                restore_transaction(release, device, args.backup_dir, identity,
                                    device.description["storage_id"], args.resume)
            else:
                install_transaction(release, bundle, device, args.backup_dir, identity,
                                    device.description["storage_id"], args.resume)
    if args.boot_after_install:
        print("DA exit acknowledged. This remote may need its side Power button to boot.")
    print("Original partition restore readback completed. Normal startup still requires verification." if args.restore else "Installation readback completed. First boot, onboarding and recovery still require verification.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-private-flash", action="store_true")
    for name in ("manifest", "baseline", "checkout", "loader", "preloader", "backup-dir"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    for name in ("loader-sha256", "preloader-sha256", "confirm-cid-sha256", "ports"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--bus", type=int, required=True)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--lock-dir", type=Path, default=Path.home()/".local/state/couch-installer/locks")
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--restore", action="store_true", help="Restore journaled originals before first boot; refuses unexpected partition changes")
    parser.add_argument("--boot-after-install", action="store_true")
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    try:
        run(args)
        return 0
    except (InstallError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"Private installation stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
