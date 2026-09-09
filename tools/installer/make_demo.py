#!/usr/bin/env python3
"""Create tiny synthetic release/device files; these cannot boot a physical remote."""
import argparse
import hashlib
import json
import os
from pathlib import Path

import couch_install as installer


def create(root):
    root = Path(root)
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    if any(root.iterdir()):
        raise installer.InstallError("Demo directory must be empty")
    release_dir, device_dir = root / "release", root / "device"
    release_dir.mkdir(mode=0o700)
    device_dir.mkdir(mode=0o700)
    names = sorted(installer.IDENTITY_PARTITIONS | installer.REQUIRED_IMAGES)
    partitions = {name: {"offset": (index + 1) * 4096, "size": 4096} for index, name in enumerate(names)}
    description = {"simulation": True, "model": installer.MODEL,
                   "storage_id": "SIMULATED-HA100-001", "partitions": partitions}
    (device_dir / "device.json").write_text(json.dumps(description, indent=2))
    for index, name in enumerate(names):
        (device_dir / f"{name}.img").write_bytes(bytes([index + 1]) * 4096)
    images = {}
    for name in installer.REQUIRED_IMAGES:
        data = bytearray(4096)
        if name in ("boot", "recovery"):
            data[:8] = b"ANDROID!"
        else:
            data[1080:1082] = b"\x53\xef"
        filename = f"{name}.img"
        (release_dir / filename).write_bytes(data)
        images[name] = {"file": filename, "sha256": hashlib.sha256(data).hexdigest()}
    manifest = {"schema": 1, "model": installer.MODEL, "partitions": partitions, "images": images}
    (release_dir / "manifest.json").write_text(json.dumps(manifest, indent=2))
    (root / "identity.json").write_text(json.dumps({"device_id": "DEMO000000000001",
                                                 "wifi_mac": "02:00:00:00:00:01",
                                                 "bluetooth_mac": "02:00:00:00:00:02"}, indent=2))
    return manifest, description


if __name__ == "__main__":
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    create(args.directory)
    print(f"Synthetic demo created at {args.directory}. Never flash these images to hardware.")
