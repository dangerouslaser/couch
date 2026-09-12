#!/usr/bin/env python3
"""Convert a Sanytron HA100 factory binpackage into the installer's strict
full-partition bundle + manifest.

The installer's DA writer requires each image to be a raw image whose size is
exactly the device GPT partition size, and it hashes the whole partition back.
Factory images are not that: system/vendor/cache/userdata are Android-sparse and
smaller than their partitions; boot/lk/logo/etc. are raw but shorter. This tool:

  * expands Android-sparse images to raw,
  * zero-pads read-only/raw partitions (system, vendor, boot, recovery, lk, lk2,
    logo, odmdtbo, secro) to the exact device partition size,
  * builds a fresh full-size empty ext4 for cache and userdata (factory reset),
  * records the sha256 of every produced full-partition image in a manifest.

It never writes the preloader (preserve the factory bootloader already on the
device) and never touches calibration partitions (proinfo/nvram/nvdata/
protect1/protect2/... are not in the produced image set).
"""
import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys
from pathlib import Path

SPARSE_MAGIC = 0xED26FF3A

# partition -> factory binpackage member. preloader intentionally excluded.
MEMBERS = {
    "lk": "lk.img",
    "lk2": "lk.img",
    "boot": "boot.img",
    "recovery": "recovery.img",
    "logo": "logo.bin",
    "odmdtbo": "odmdtbo.img",
    "secro": "secro.img",
    "vendor": "vendor.img",
    "system": "system.img",
    "cache": "cache.img",
}
# cache's factory sparse image expands to the full partition as a valid ext4.
# userdata is handled separately: the factory userdata member is only a sparse
# placeholder (not a real filesystem). This device formats /data as F2FS, so we
# flash a known-good full-size empty F2FS userdata supplied via --userdata-image.
FRESH_EXT4 = {"cache"}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def is_sparse(path):
    with open(path, "rb") as f:
        return struct.unpack("<I", f.read(4))[0] == SPARSE_MAGIC


def expand_sparse(src, dst):
    """Expand an Android sparse image to a raw image. Returns raw byte length."""
    with open(src, "rb") as f, open(dst, "wb") as out:
        hdr = f.read(28)
        (magic, major, minor, file_hdr_sz, chunk_hdr_sz, blk_sz,
         total_blks, total_chunks, _crc) = struct.unpack("<IHHHHIIII", hdr)
        if magic != SPARSE_MAGIC:
            raise ValueError("not a sparse image")
        if major != 1:
            raise ValueError(f"unsupported sparse major version {major}")
        if file_hdr_sz != 28 or chunk_hdr_sz != 12:
            raise ValueError("unexpected sparse header sizes")
        written_blocks = 0
        for _ in range(total_chunks):
            chunk_type, _resv, chunk_blks, total_sz = struct.unpack("<HHII", f.read(12))
            data_sz = total_sz - 12
            if chunk_type == 0xCAC1:  # RAW
                remaining = chunk_blks * blk_sz
                if data_sz != remaining:
                    raise ValueError("raw chunk size mismatch")
                while remaining:
                    buf = f.read(min(1 << 20, remaining))
                    if not buf:
                        raise ValueError("truncated raw chunk")
                    out.write(buf)
                    remaining -= len(buf)
            elif chunk_type == 0xCAC2:  # FILL
                fill = f.read(4)
                block = fill * (blk_sz // 4)
                for _i in range(chunk_blks):
                    out.write(block)
            elif chunk_type == 0xCAC3:  # DONT_CARE
                out.write(b"\x00" * (chunk_blks * blk_sz))
            elif chunk_type == 0xCAC4:  # CRC32
                f.read(4)
            else:
                raise ValueError(f"unknown sparse chunk type {chunk_type:#x}")
            written_blocks += chunk_blks
        if written_blocks != total_blks:
            raise ValueError(f"block count mismatch: {written_blocks} != {total_blks}")
        return total_blks * blk_sz


def pad_to(path, size):
    cur = os.path.getsize(path)
    if cur > size:
        raise ValueError(f"{path} ({cur}) larger than partition size {size}")
    with open(path, "r+b") as f:
        f.truncate(size)


def fresh_ext4(raw, size, name):
    """Grow an expanded factory ext4 to the full device partition size so it is a
    valid, mountable, empty filesystem of the right geometry (factory reset)."""
    with open(raw, "r+b") as f:
        f.truncate(size)
    subprocess.run(["e2fsck", "-fy", raw], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    block = 4096
    if size % block:
        raise ValueError(f"{name} size not a multiple of {block}")
    r = subprocess.run(["resize2fs", raw, f"{size // block}"],
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if r.returncode != 0:
        raise RuntimeError(f"resize2fs {name} failed: {r.stdout.decode(errors='replace')}")
    if os.path.getsize(raw) != size:
        raise ValueError(f"{name} not full size after resize: {os.path.getsize(raw)} != {size}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--binpackage", type=Path, required=True, help="extracted factory binpackage dir")
    ap.add_argument("--device-manifest", type=Path, required=True,
                    help="trusted device manifest (partition sizes authority)")
    ap.add_argument("--archive", type=Path, required=True, help="the source factory zip (for provenance hash)")
    ap.add_argument("--version", required=True, help="factory firmware version string")
    ap.add_argument("--userdata-image", type=Path, default=None,
                    help="optional known-good full-size empty F2FS userdata. Omit to leave /data "
                         "untouched: a factory restore need not rewrite userdata, since Android "
                         "formats /data on first boot, and writing an empty image just copies "
                         "gigabytes of mostly-zero bytes over the slow download agent.")
    ap.add_argument("--out", type=Path, required=True, help="output bundle dir")
    args = ap.parse_args()

    manifest = json.loads(args.device_manifest.read_text())
    parts = manifest["partitions"]
    args.out.mkdir(parents=True, exist_ok=True)
    os.umask(0o077)

    images = {}
    for name, member in MEMBERS.items():
        if name not in parts:
            print(f"!! {name} not in device manifest partitions; skipping", file=sys.stderr)
            continue
        size = parts[name]["size"]
        src = args.binpackage / member
        if not src.is_file():
            raise SystemExit(f"missing factory member {member}")
        out_img = args.out / f"{name}.img"
        # Stage raw content.
        if is_sparse(src):
            raw_len = expand_sparse(src, out_img)
            print(f"{name}: expanded sparse {member} -> {raw_len} bytes", flush=True)
        else:
            with open(src, "rb") as s, open(out_img, "wb") as d:
                for block in iter(lambda: s.read(1 << 20), b""):
                    d.write(block)
            print(f"{name}: raw {member} -> {os.path.getsize(out_img)} bytes", flush=True)
        # Size to the exact device partition.
        if name in FRESH_EXT4:
            fresh_ext4(out_img, size, name)
            print(f"{name}: grown to full empty ext4 {size} bytes", flush=True)
        else:
            pad_to(out_img, size)
        if os.path.getsize(out_img) != size:
            raise SystemExit(f"{name}: produced {os.path.getsize(out_img)} != partition {size}")
        digest = sha256_file(out_img)
        images[name] = {"file": f"{name}.img", "sha256": digest, "size": size}
        print(f"{name}: sha256 {digest}", flush=True)

    # userdata: full-size empty F2FS (factory reset of /data), supplied externally.
    # Optional: omitted by default so the restore does not copy gigabytes of empty
    # filesystem; Android formats /data on first boot.
    if args.userdata_image is not None and "userdata" in parts:
        size = parts["userdata"]["size"]
        if args.userdata_image.stat().st_size != size:
            raise SystemExit(f"userdata image {args.userdata_image.stat().st_size} != partition {size}")
        out_img = args.out / "userdata.img"
        with open(args.userdata_image, "rb") as s, open(out_img, "wb") as d:
            for block in iter(lambda: s.read(1 << 20), b""):
                d.write(block)
        with open(out_img, "rb") as f:
            f.seek(1024)
            sb = struct.unpack("<I", f.read(4))[0]
        if sb != 0xF2F52010:
            raise SystemExit(f"userdata is not F2FS (superblock magic {sb:#x})")
        digest = sha256_file(out_img)
        images["userdata"] = {"file": "userdata.img", "sha256": digest, "size": size}
        print(f"userdata: F2FS empty, sha256 {digest}", flush=True)

    release = {
        "schema": 1,
        "model": manifest["model"],
        "purpose": "factory-restore",
        "private_candidate": True,
        "installable": False,
        "source": {"archive": args.archive.name,
                   "archive_sha256": sha256_file(args.archive),
                   "archive_size": os.path.getsize(args.archive),
                   "version": args.version},
        "preserved": ["preloader", "proinfo", "nvram", "nvdata", "protect1", "protect2",
                      "seccfg", "para", "expdb", "frp", "metadata", "oemkeystore", "keystore"],
        "partitions": {k: {"offset": v["offset"], "size": v["size"]} for k, v in parts.items()},
        "images": images,
    }
    (args.out / "release.json").write_text(json.dumps(release, indent=2, sort_keys=True))
    print("wrote", args.out / "release.json", flush=True)


if __name__ == "__main__":
    main()
