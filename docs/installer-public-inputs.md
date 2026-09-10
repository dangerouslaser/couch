# Inputs for installing on an original Android HA100

This audit is offline. It does not enable the public installer, alter USB
transport policy, or demonstrate installation on a previously unmodified unit.

## Preserve identity without an existing Couch baseline

Sanytron asks owners to save Device ID, Wi-Fi MAC and Bluetooth MAC before
flashing, using the Android settings screens. Its guide documents user-visible
values, not partition offsets or a binary decoder. See the
[official identity-backup instructions](https://hub.sanytron.com/support/astrion/backup-device-identity).

The current private installer compares an earlier Couch runtime baseline. A
fresh Android unit needs a separate first-capture workflow: identify the model,
read both GPT copies, retain the fixed-format DA storage identity, and save all
five protected partitions (`proinfo`, `nvram`, `nvdata`, `protect1`, `protect2`)
with independent readback before any persistent write. That establishes a new
opaque preservation baseline; it must not claim that Device ID/MAC fields have
been decoded or matched. Never manufacture an expected runtime baseline by
copying the device's current values into a record labeled previously verified.

Confirmed stock evidence:

- Launcher 1.4.2 `SettingAboutActivity` displays `Build.getSerial()`; its
  `formatSerial` method only inserts spaces. `MachineUtils` also reads
  `ro.serialno`, but has development fallback logic and is not the primary
  About-screen source.
- Stock `init.mt6580.usb.rc` writes `${ro.serialno}` into Android USB `iSerial`
  and `${ro.product.model}` into `iProduct` on boot. Reading the normal Android
  USB serial before reboot may provide an ADB-free Device ID capture. This
  needs physical validation and matching to the subsequent selected USB port;
  the preloader descriptor is not assumed to carry the same serial.
- Stock `libcustom_nvram.so` names `/vendor/nvdata/APCFG/APRDEB/WIFI`,
  `BT_Addr`, and `PRODUCT_INFO`. Init creates/mounts the nvdata runtime path.
  Saved nvdata filesystem metadata contains `WIFI` at 1026 bytes and `BT_Addr`
  at 66 bytes. No identifier bytes were printed during this audit.
- The donor Wi-Fi driver describes a 512-byte configuration structure with
  two 16-bit version fields before its MAC field. That does **not** establish
  the complete 1026-byte stock file format. Device ID's raw partition mapping,
  Bluetooth record layout and exact stock NVRAM version/checksum handling
  still require validation. Do not guess fixed offsets in whole partitions.

A first public flow can preserve raw identity backups while asking the owner
to save/confirm the three original Android values. Automatic field decoding
should be added only with versioned format evidence and comparisons against
original Android values. A missing value must remain explicitly unknown.

## Approved EMI without a per-unit preloader backup

The model-specific stock app firmware API returned version `2026020212` and
[its firmware archive](http://cdn.lifex360.com/attach/2026/03/c8znx1jxwhjr2kcv08t4.zip).
The metadata endpoint recovered from the app is
`http://gateway.dev.aiks360.com/api/v1/base-service/pub/checkAppVersionV2?_allow_anonymous=true`,
queried for `aiks_ha_remote_ha100.fw` without device identity.

| Input | Verified value |
|---|---|
| Complete ZIP | 518937104 bytes; SHA-256 `e9d6cd18a43c9195caac613a6168c43cf9b95758a7348931b17ae3aec0aa863a` |
| `preloader.img` member | 109992 bytes; SHA-256 `0ad0d14b7203d98a6567af7a022cfe5df5b6fcbba60cb4e9b4bc2ee569cf1069` |
| Extracted EMI | version 21; 752 bytes; SHA-256 `326b085f4a7917ea9e1f6271a8de86eb1a064097631750e3fc18e8d25577125d` |

The official member's EMI is byte-identical to the same-device preloader EMI
used in the validated private DA sessions. Extraction used the exact offline
`DAconfig.m_extract_emi` function at mtkclient revision
`60e07f3b343a4469389f15967626d63e049968d4`, from
`mtkclient/Library/DA/daconfig.py`. Its legacy DA consumer is
`mtkclient/Library/DA/legacy/dalegacy_lib.py`.

This offers a bootstrap path: the owner supplies/downloads the pinned official
ZIP, the host verifies it and extracts only approved EMI before DA upload.
There is no need to flash a preloader. Do not assume compatibility across HA100
memory/board revisions or enable upstream's heuristic preloader directory scan.
The separately hash-approved download agent and its source/authentication policy
remain required. Uploaded EMI equivalence still needs a controlled RAM-only
hardware check using the official member instead of the saved per-unit input.

The HTTP metadata field named `signature` is an MD5-style digest, not verified
publisher signing. The pinned SHA-256 detects changed bytes but does not itself
establish authenticated distribution. Keep the source-authentication and
redistribution reviews separate from successful extraction.

## Assemble vendor runtime locally from the official ZIP

All **33** files in the previously verified private runtime bundle match the
same official OTA exactly: 18 vendor files and 15 system files. This includes
WMT loaders, firmware/modules, the bionic linker/libraries and required linker
and SELinux property configuration. It is feasible to assemble these files on
the owner's machine instead of redistributing Couch's copy of them.

`tools/release/official_runtime.py` now implements the offline step:

```sh
# Run on Ollie/Linux with Python brotli and debugfs available.
python3 tools/release/official_runtime.py /path/to/official-fw.zip \
  /path/to/new-private-vendor-directory
```

`ha100_official_runtime.json` pins archive, selected ZIP members, reconstructed
system/vendor images and every runtime file. The tool never downloads, mounts,
opens a device, or performs general ZIP extraction. It accepts only bounded,
nonoverlapping full-OTA `new`/`zero`/`erase` transfer lists, verifies compressed
member hashes, bounds decompressed output to the declared ranges, and verifies
reconstructed image hashes before reading the allowlisted files with debugfs.
Temporary images are private and removed afterward. Final output is published
to the new directory only after all file hashes and inventory checks pass.

Output uses the existing private-vendor bundle format and works with
`prepare_private_rootfs.py`; `private_only` stays true, `installable` stays
false, and `redistribution_authorized` stays false. The real pinned ZIP produced
all 33 matching files on Ollie. Unit tests cover range overlap/bounds, forbidden
commands, stash rejection, ZIP member hashes/symlinks, truncation/expansion,
wrong archive hashes and existing-output refusal. Real Brotli tests run on
Ollie where that dependency is installed.

Download/bootstrap UX, independent model recognition, fresh-device identity
reporting, approved loader delivery, complete corresponding-source notices,
release authentication and actual fresh-Android end-to-end testing remain
separate work. None of these observations enable a public install gate.
