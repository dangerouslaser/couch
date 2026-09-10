# Private installer candidate validation

The September 10 candidate is prepared on Ollie at
`~/backups/couch-release-inputs/e2e-20260910/bundle/release.json`.
It is exclusively for the owner's controlled end-to-end test. The manifest has
`private_candidate: true`, `installable: false`, and
`redistribution_authorized: false`. No image or private vendor archive belongs
in Git or a public download.

## Candidate contents

| Image | Raw bytes | SHA-256 |
|---|---:|---|
| boot.img | 16777216 | `393be1ddfae7541d74189c314266042fadf1659c292e7362f916999a574db11d` |
| recovery.img | 16777216 | `b212c4a9036abb89e514cd2c32e250777899434b14e845cc1e4773ef35196635` |
| userdata.img | 5905055744 | `60bb36a87b85498de69694d6d14468f5a93861cab7ec2d05a29da9513c606763` |

Boot reuses the exact tested `ea122a39` normal zImage. Recovery's zImage hash
`d280c4051efd0a24f80f26e7e0bf62f1e1621e2668cf3f4ddabfd4c8c686648e`
matches the original Android backup, preserving independent recovery code.
Both use newly generated clean ramdisks: current tracked init, BusyBox, fbcon,
and boot-health script for normal boot only. No developer `extra/` directory,
saved Wi-Fi, keys, enrollment or identity bundle is copied into them.

Userdata contains the verified private vendor overlay and refreshed runtime:
GUI `c850911cb54fbd5baf7421f1e0382de66ac24b8298c076675591e2b99f3e68b8`,
daemon `2f1aab33fac3af3476b5c775af19018456b102cddb9a02c63373829f46f7b6eb`.
The GUI includes the acknowledged local KEYLOG diagnostic hunk. The archive
contains empty onboarding configuration, not the owner's saved rooms/network.
These hashes identify tested binaries; they do not claim a clean reproducible
source-to-binary build attestation.

## Reproduce private preparation on Ollie

The candidate's `runtime-checkout/` preserves the exact allowlisted input files
and updated `build/staging-e2e.json`. Pinned ARM packages and filesystem tools
are preserved under `caches/arm-packages` and `caches/image-tools` beside it,
with byte-identical closure manifests from the verified original caches.
Run from `runtime-checkout/`, using fresh output directories:

```sh
python3 tools/release/prepare_rootfs.py build/staging-e2e.json \
  ../caches/arm-packages ../packaged-new
python3 tools/release/prepare_private_rootfs.py ../packaged-new \
  ../../vendor-verified ../rootfs-private-new
python3 tools/release/prepare_ext4.py ../rootfs-private-new \
  ../caches/image-tools tools/release/ha100_userdata_geometry.json \
  ../userdata-new --private-vendor-bundle ../../vendor-verified
python3 tools/release/prepare_boot_candidates.py \
  --normal-template ../couch-board-init-fixed.img \
  --stock-recovery-template ../linux-recovery-KNOWNGOOD.img \
  --kernel-manifest ../board-init-manifest.json --output ../boot-images-new
```

The boot helper verifies the pinned normal kernel, preserves each template's
DTB/header, generates ramdisks from an explicit file set, and pads each image
to 16 MiB. It rejects an existing output directory and shared boot/recovery
kernels. It never accesses USB or a block device. Its output remains private
and noninstallable. Retain independent image/ramdisk provenance alongside the
private release manifest and full observed partition layout.

## Completed preparation checks

Package/vendor archive safety and hashes passed; userdata passed offline
`e2fsck -fn`, size and legacy-ext4 feature checks. The runtime archive contains
the expected executable hashes and empty onboarding configuration. Two clean
boot/recovery generations were byte-identical. Two independent full-size
userdata generations also produced the same image SHA-256. The installer core accepted
all three full-partition image lengths, hashes and observed layout metadata.
Host tests cover clean boot payloads, independent recovery kernels, output
collision rejection and exact image sizing.

## Required physical and public-release gates

The newly generated clean ramdisks and refreshed userdata have not yet completed
physical installation. Root's controlled test must re-read and compare actual
CID, complete layout and protected identity/calibration hashes, save full
current userdata and boot/recovery backups, then use the explicitly private
installer path. Existing settings have a separate private backup. Keep USB
recovery available; clean onboarding will require configuring Wi-Fi and
re-enrolling developer access if wanted.

Validate installation readback, cold boot, advancing GUI/BCB clearance,
independent recovery, onboarding, display/touch/buttons, Wi-Fi, charging/wake,
and IR. A successful private test does not authorize redistribution of vendor
binaries or replace signed releases, BusyBox/kernel corresponding-source and
license review, clean binary build attestation, or transport fault/recovery
coverage. Public curl/TUI delivery must retain those release gates.
