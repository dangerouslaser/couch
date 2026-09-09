# Clean release staging

`tools/release/clean_stage.py` assembles a deterministic **noninstallable** rootfs staging archive. It takes a hash-pinned clean Alpine ARMv7 minirootfs and an explicit list of Couch artifacts. It never copies a running remote's rootfs, extracts archive links onto the host, or includes vendor files automatically.

The independent staging manifest is not an installer release manifest. It records source commit, input hashes, normalized timestamps, selected artifacts, and output hash. `installable` stays false: offline packages, reviewed vendor inputs, partition images, release signatures, and hardware recovery validation remain required.

Example input (replace the source commit and verify every input hash):

```json
{
  "schema": 1,
  "source_commit": "FULL_40_CHARACTER_GIT_COMMIT",
  "source_date_epoch": 1788912000,
  "alpine": {
    "path": "build/alpine-minirootfs-3.21.7-armv7.tar.gz",
    "version": "3.21.7",
    "architecture": "armv7",
    "sha256": "762c2e1e8d3e6f02e75754de8e4e5766f4c1b280d42f3d640fb30f810e786ee8"
  },
  "artifacts": []
}
```

The recorded base hash matches Alpine's [published checksum](https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/armv7/alpine-minirootfs-3.21.7-armv7.tar.gz.sha256). Hash matching alone is not publisher authentication. The source commit is an inventory field; reproducible source builds and provenance still need release-pipeline enforcement.

Each artifact specifies `source` relative to the checkout, `destination` inside the image, `sha256`, and mode `493` (0755) or `420` (0644). Only supported `/opt/couch` executable/script names and web assets are accepted. Do not point the tool at device backups, a mounted userdata partition, or a sanitized copy of a developer rootfs.

```sh
python3 tools/release/clean_stage.py build/release-staging-input.json build/clean-stage
python3 -m unittest discover -s tools/release -v
```

The output directory must be new. Private-key paths, saved networks, private Couch configuration, password hashes, path traversal, special devices, and files below archive symlinks are rejected. The base BusyBox must be ARM32 ELF. The archive gets a minimal empty Couch configuration, hostname, and key-only SSH settings; root has no usable password. Existing enrollment-gated `stage2/sshd.sh` generates host keys on first authorized use. No keys or enrollment are created by staging.

Archive ownership and timestamps are normalized. Absolute symlinks are retained as image-root links because Alpine uses them extensively; no host extraction occurs. Downstream image assembly must likewise avoid following links outside the image. Fixed Python/zlib and input artifacts are needed for reproducibility across build hosts. Filename/content checks reduce accidental secret inclusion; they cannot prove arbitrary supplied binaries contain no embedded private data.

Validation includes traversal/link attacks, private state and keys, wrong hashes/version, empty onboarding configuration, reproducible output, and the existing official 3.21.7 ARMv7 minirootfs (525 entries after generated directories/files). This is not yet a bootable release: do not flash its tarball.
