#!/usr/bin/env python3
"""Private read-only legacy/stream DA benchmark. No flash writes or USB resets."""
import argparse
import json
from pathlib import Path
import sys
import time

from capture_readonly import baseline_record, compare_baseline, wait_preloader
from couch_install import CHUNK, IDENTITY_PARTITIONS, InstallError, REPO, require, save_json
from mtk_readonly import ConnectedMtkReader
from mtk_session import connected_session, loader_bytes, source_pin
from mtk_stream_reader import ConnectedMtkStreamReader
from mtk_usb import ExactUsbBackend


def validate(args):
    try:
        ports = tuple(int(part) for part in args.ports.split("."))
    except ValueError as error:
        raise InstallError("Invalid USB port path") from error
    require(args.bus > 0 and ports and all(part > 0 for part in ports), "Invalid USB bus/port path")
    require(0 < args.timeout <= 600, "Timeout must be between 1 and 600 seconds")
    baseline = baseline_record(args.baseline)
    require(baseline is not None and "cid" in baseline
            and set(baseline.get("identity_sha256", {})) == IDENTITY_PARTITIONS,
            "Benchmark requires full runtime CID/layout and all five identity hashes")
    require(0 < baseline["partitions"]["proinfo"]["size"] <= 4 * CHUNK,
            "Checksum probe requires proinfo at most 4 MiB")
    require(0 < baseline["partitions"]["nvdata"]["size"] <= 64 * CHUNK,
            "Benchmark partition must be at most 64 MiB")
    report = getattr(args, "report", None)
    if report is not None:
        report = report.absolute()
        require(not report.is_symlink() and not report.exists(), "Use a new benchmark report path")
        require(not report.resolve().is_relative_to(REPO), "Keep benchmark reports outside the repository")
        require(report.parent.is_dir(), "Benchmark report parent must exist")
    source_pin(args.checkout)
    loader_bytes(args.loader, args.loader_sha256)
    loader_bytes(args.preloader, args.preloader_sha256)
    return baseline, ports, report


def timed_hash(reader, name, expected, *, clock=time.monotonic):
    print(f"Reading {name} with {reader.description['transport']}…", flush=True)
    size = reader.description["partitions"][name]["size"]
    before = dict(getattr(reader, "stats", {}))
    at = clock()
    checksum = reader.hash(name)
    elapsed = clock() - at
    require(elapsed > 0, "Invalid benchmark clock interval")
    require(checksum == expected, f"{name} readback differs from independently verified baseline")
    stats = {key: value - before.get(key, 0) for key, value in getattr(reader, "stats", {}).items()}
    result = {"partition": name, "bytes": size, "seconds": elapsed,
              "mib_per_second": size / CHUNK / elapsed, "baseline_hash_verified": True,
              "stats": stats}
    print(f"{name}: verified {size} bytes in {elapsed:.3f}s ({result['mib_per_second']:.2f} MiB/s)", flush=True)
    if stats:
        print("Stream timing: " + json.dumps(stats, sort_keys=True), flush=True)
    return result


def benchmark(args, *, session=connected_session, enumerate_devices=None,
              legacy_factory=ConnectedMtkReader, stream_factory=ConnectedMtkStreamReader):
    baseline, ports, report_path = validate(args)
    if args.check_only:
        print("Pinned inputs and complete benchmark baseline verified. No USB operation performed.")
        return None

    def candidate_provider(enumerate_backend):
        print(f"Waiting on USB bus {args.bus}, port {args.ports}, preloader PID 2000. No reset will be sent.", flush=True)
        return wait_preloader(enumerate_devices or enumerate_backend, args.bus, ports, args.timeout)

    def devices(mtk, revision):
        legacy = legacy_factory(mtk, revision)
        print("Validating complete runtime identity through the established legacy reader first…", flush=True)
        compare_baseline(legacy, baseline, progress=lambda message: print(message, flush=True))
        stream = stream_factory(mtk, revision)
        for key in ("storage_id", "runtime_cid_sha256", "capacity", "partitions"):
            require(stream.description[key] == legacy.description[key], "Stream reader observed a different device/layout")
        return legacy, stream

    factory = lambda checkout: ExactUsbBackend(checkout, preloader=args.preloader,
                                               preloader_sha256=args.preloader_sha256)
    result = {"schema": 1, "complete": False, "writes_sent": False, "baseline_verified": False,
              "checksum_probe_verified": False, "usb_cleanup_verified": False,
              "boot_requested": args.boot_after_capture, "boot_acknowledged": False,
              "normal_os_verified": False}
    with session(args.checkout, args.loader, args.loader_sha256, args.lock_dir, None, factory,
                 candidate_provider=candidate_provider, device_factory=devices,
                 boot_after_capture=args.boot_after_capture) as (legacy, stream):
        result["baseline_verified"] = True
        # A checksum mismatch stops inside the stream before data acknowledgement;
        # never fall back or start another protocol on that poisoned session.
        result["probe"] = timed_hash(stream, "proinfo", baseline["identity_sha256"]["proinfo"])
        result["checksum_probe_verified"] = True
        result["legacy"] = timed_hash(legacy, "nvdata", baseline["identity_sha256"]["nvdata"])
        result["stream"] = timed_hash(stream, "nvdata", baseline["identity_sha256"]["nvdata"])
        result["speed_ratio"] = result["legacy"]["seconds"] / result["stream"]["seconds"]
        print(f"Verified stream/legacy speed ratio: {result['speed_ratio']:.2f}x. This is one read-only trial, not an installer release.", flush=True)
    result["usb_cleanup_verified"] = True
    result["boot_acknowledged"] = args.boot_after_capture
    result["complete"] = True
    if report_path is not None:
        save_json(report_path, result)
    print("Read-only benchmark complete. No flash writes sent.", flush=True)
    if args.boot_after_capture:
        print("DA exit acknowledged; the tested HA100 powers off. Hold Power to start Couch. OS startup is not verified.", flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("checkout", "loader", "preloader", "baseline"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    for name in ("loader-sha256", "preloader-sha256", "ports"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--bus", type=int, required=True)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--lock-dir", type=Path, default=Path.home() / ".local/state/couch-installer/locks")
    parser.add_argument("--report", type=Path, help="New JSON report path outside the repository; contains timings, not raw identity")
    parser.add_argument("--boot-after-capture", action="store_true", help="Request DA exit only after all hashes match; tested device then needs manual Power")
    parser.add_argument("--check-only", action="store_true")
    try:
        benchmark(parser.parse_args())
    except (InstallError, OSError, ValueError, KeyError) as error:
        print(f"Benchmark stopped: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("Benchmark interrupted; no reset or flash write was requested. Discard this DA session.", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
