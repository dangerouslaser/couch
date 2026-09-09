# Couch USB installer

Status: experimental installer policy engine, simulation CLI, and read-only connected-session adapter. **This does not yet install Couch on a physical remote.** The `install` command refuses to run. No live USB session, flash operation, recovery boot, or identity extraction has been validated for this implementation.

## Intended experience

The eventual command is `couch-install <release>`. It should wait for the attached HA100 to reboot into MediaTek download mode, identify the unit, back up its identity and original partitions, install the recovery image and OS, verify readback, then guide first boot. First boot should use the remote's keyboard and Wi-Fi scan/password/test flow, then show the webUI address for configuration. An access point should be an explicit recovery option, not an installer prerequisite. A release should contain everything needed offline; users should not need Android debugging, SSH, Docker, a compiler, or a running Android installation.

The implementation is Python 3.10+ because the repository's image tooling and the prospective mtkclient transport are Python. It never builds a kernel. Release kernels continue to be built on Ollie.

## Research and hardware constraints

Sanytron requires recording **Device ID, Wi-Fi MAC, and Bluetooth MAC before flashing**. Its guide gives menu locations but no binary offsets or USB API for extracting these fields. Preserve those values exactly; the checklist says 16 serial characters, while its example has 15. Automatic decoding needs validation against the same device's displayed values. [Sanytron identity backup](https://hub.sanytron.com/support/astrion/backup-device-identity)

Sanytron's supported flashing instructions use Windows, MTK drivers and SP Flash Tool with a scatter file. The tool waits before a powered-off remote is connected. This establishes download-mode flashing as a supported recovery route; it does **not** establish that an ordinary reboot is reliably catchable by our installer. [Sanytron flashing instructions](https://hub.sanytron.com/support/astrion/flash-firmware)

The repository reports a transient MediaTek preloader CDC port around six seconds into boot. Its working runtime/recovery serial gadget is `0e8d:201c`, which is deliberately excluded from download-mode discovery. Generic MediaTek IDs are candidates only: neither VID/PID nor a USB bus address identifies an HA100 or its owner. [Repository recovery notes](../README.md), [upstream USB identifiers](https://github.com/bkerler/mtkclient/blob/main/mtkclient/config/usb_ids.py)

mtkclient documents partition reads/writes, GPT inspection, stock mode and scripts that execute several commands in a session. Its installation documentation covers Linux and macOS; Windows has separate driver instructions. A hardware adapter must pin a tested upstream revision and compatible download agent, maintain one session, obtain stable silicon/storage identification, and validate the actual layout. Do not implement it by scraping human console logs or launching a fresh USB handshake for every partition. [mtkclient usage](https://github.com/bkerler/mtkclient/blob/main/README-USAGE.md), [installation](https://github.com/bkerler/mtkclient/blob/main/README-INSTALL.md), [Windows setup](https://github.com/bkerler/mtkclient/blob/main/README-WINDOWS.md)

## What is implemented

`tools/installer/couch_install.py` contains a transport-independent installation sequence exercised through a regular-file device simulator:

1. Check release model, complete nonoverlapping partition layout, image size, boot/ext4 magic and SHA-256. Compare every observed partition's byte offset and size with the release profile. Raw, full-partition images only; Android sparse images and repartitioning are rejected.
2. Bind the journal to the exact release, simulated storage identity/layout, and recorded identity. Require the caller's explicit target identifier.
3. Back up `proinfo`, `nvram`, `nvdata`, `protect1`, `protect2`, plus every partition to be overwritten. Save the recorded Device ID/MACs and observed layout. Compare each complete backup with an independent device read before permitting writes. Reserve sufficient disk space for all originals.
4. Write and read back recovery first, then userdata, optional logo/odmdtbo, and boot last. Required images are recovery, userdata and boot. Every other partition is outside the write allowlist—including `preloader_*`, `lk`, `lk2`, `para` and all calibration partitions. No whole-device format, erase command, or automatic reboot exists.
5. Recheck identity partitions at completion. Preserve a journal with each write's `writing`/`verified` state. Explicit resume verifies all backups, recorded layout, identity, completed writes and untouched targets before continuing; it never replaces original backups with newly installed images. Every release image is checked again before the first resumed write, so a damaged later image cannot cause a partially executed plan.

Backups are outside the repository, directory mode 0700 and file mode 0600. They are **not encrypted**. Store them privately and retain a second copy before eventual hardware flashing. An interrupted backup requires a new destination; an interrupted write can resume from its completed backup set. A mismatch in an already verified partition stops rather than silently overwriting an unexpected change. The journal is fsynced and replaced atomically; a stale `.tmp` after interruption currently requires inspection before removal.

This backs up overwritten OS partitions as well as identity, but is not a complete factory-flash archive. Automatic rollback is not implemented. A future restore command must validate the same physical unit before restoring its originals. The existing recovery USB shell can operate without a working userdata filesystem; recovery networking depends on userdata. Existing BCB/watchdog behavior does not guarantee automatic recovery from every interruption or failure before init runs. [Current recovery implementation](../recovery/init)

## Try the simulation

No dependencies beyond Python are required. These commands create **synthetic, nonbootable** files in a temporary directory:

```sh
DEMO_DIR=$(mktemp -d /tmp/couch-installer-demo.XXXXXX)
python3 tools/installer/make_demo.py "$DEMO_DIR"
python3 tools/installer/couch_install.py plan \
  --manifest "$DEMO_DIR/release/manifest.json" \
  --device-dir "$DEMO_DIR/device"
python3 tools/installer/couch_install.py simulate \
  --manifest "$DEMO_DIR/release/manifest.json" \
  --device-dir "$DEMO_DIR/device" \
  --backup-dir "$DEMO_DIR/originals" \
  --identity "$DEMO_DIR/identity.json" \
  --confirm-device SIMULATED-HA100-001
python3 -m unittest discover -s tools/installer -v
```

The `plan` command is a dry run and writes nothing. Repeat `simulate` with `--resume` to verify a completed journal or continue after a simulated interruption. These tests cover wrong targets/layouts, prohibited writes, bad hashes, backup/readback failures, partial writes, corrupted backups, identity changes, write ordering and resume.

The reviewed suite has 40 tests, including CLI checks that `plan` leaves all fixture files unchanged and `install` refuses before constructing a transport. Adapter tests cover GPT CRCs and disagreement between valid copies, bounded reads, short responses, target confirmation, and independent identity readback. Session-gate tests cover source/loader changes, competing locks, ambiguous device selection and cleanup. Simulation is single-process only; it does not yet use the session gate's lock. Do not treat the simulator as a hardware-ready transaction manager.

Optional descriptor-only discovery requires PyUSB and a libusb backend. Install them in a development virtual environment following the host's package instructions, then run:

```sh
python3 tools/installer/couch_install.py watch-usb --timeout 60
```

It polls generic MTK descriptors without claiming interfaces, detaching drivers, starting a download agent or sending a flash command. **Observation does not hold the boot window open.** No USB discovery was run against the physical remote during this work.

## Release format and work remaining

The generated demo illustrates schema 1: `model`, `partitions` mapping names to byte `offset`/`size`, and `images` mapping writable names to bundle-local `file` and `sha256`. It is a policy demonstration, not a production layout profile. The simulator's device description includes `simulation: true`, a stable `storage_id`, model and observed layout; a real adapter must obtain these from hardware, never trust a user-supplied description.

Before enabling physical installation:

- Validate a pinned MTK transport/download agent on HA100: reboot capture timing, powered-off fallback, multiple-device rejection, stable storage identity, read-only layout and identity backups. Linux is the first proposed validation host; macOS and Windows need their own USB/driver tests. No host is yet certified for Couch installation.
- Decode Device ID/MACs from backups without modifying calibration, or collect them from Android before reboot. Until verified, require the values recorded from the device; do not guess offsets or use another remote's identity.
- Produce reproducible, clean release rootfs images with no developer SSH keys, host keys, network credentials, pairings or personal configuration. Confirm vendor-blob redistribution rights. Provide sufficient offline packages and first-boot key generation.
- Capture approved stock layouts by hardware revision and pin compatible boot/recovery/overlay combinations. The optional overlay affects both boot slots; recovery must remain bootable with it. Do not distribute the development device's image as a universal release.
- Add authenticated release manifests, trustworthy signing-key distribution, download verification and storage-size handling. SHA-256 here detects corruption, **not publisher authenticity**. Full-size raw images are intentionally conservative and inefficient; sparse/ext4 expansion needs a separately verified implementation.
- Exercise unplug/reconnect and recovery on a dedicated test unit, then implement same-device restore and post-boot identity/health verification. Protect the entire transaction against concurrent installer sessions. Only after those checks should `install` be connected to hardware writes.

## Read-only adapter progress

`tools/installer/mtk_readonly.py` now implements `ConnectedMtkReader` for an **already established** MTK download-agent session. It checks MT6580/eMMC metadata, records a storage identifier derived from the observed CID, validates both GPT headers and entry arrays with CRCs, requires matching copies, and bounds reads to the observed user-storage partitions. `backup_identity` writes private identity/calibration backups and verifies each with a second read. Its report explicitly says the model is unverified and identity fields have not been decoded. The storage identifier is not the vendor Device ID.

The adapter exposes no write, erase, reboot, loader-upload or USB-discovery method. The caller owns and must close the existing session, including after constructor failures. It is not connected to `install`, and passing a revision string does not authenticate imported Python code or a download agent.

The existing Ollie checkout was inspected at upstream commit `60e07f3b343a4469389f15967626d63e049968d4`; no USB operation was run. Its legacy in-memory reader calculates remaining bytes incorrectly after the first packet. The adapter limits calls to one 1 MiB packet and requires exact response lengths. Its independent output path also avoids the upstream asynchronous file writer. These are source-level compatibility checks, not hardware validation. [Reviewed legacy reader](https://github.com/bkerler/mtkclient/blob/60e07f3b343a4469389f15967626d63e049968d4/mtkclient/Library/DA/legacy/dalegacy_lib.py#L1131)

Do not call upstream `configure_da` as though it were passive discovery: it can upload a loader, attempt security handling and reset paths. Those operations must be reviewed and controlled before the connection harness is enabled. [Reviewed session setup](https://github.com/bkerler/mtkclient/blob/60e07f3b343a4469389f15967626d63e049968d4/mtkclient/Library/DA/mtk_da_handler.py#L136)

## Controlled session gate

`tools/installer/mtk_session.py` adds the context-managed `read_session` API. Before allowing a backend to claim USB it checks the reviewed Git revision, compares every package Python source with its committed blob, rejects extra Python source files, checks loaded module paths/source hashes, and verifies the selected loader's SHA-256. It retains the verified loader bytes for the backend. The source-only check passed against all **96 Python files** in Ollie's existing checkout; it did not import upstream code or communicate with USB.

The gate holds a private POSIX advisory lock for the full session. Cooperating processes must use the same lock directory; it cannot lock out unrelated flashing software. It requires exactly one generic MTK candidate, matches the explicit bus/address/port path, and checks the claimed device again before starting a handshake. Cleanup calls `close(reset=False)` after failures or normal completion. Linux/macOS locking is implemented; Windows locking is not.

The backend contract separates descriptor enumeration, exact-device claiming, and `start_readonly`. Its policy forbids flash writes, reset, security bypass, reconnection and watchdog changes. Loading a DA into RAM remains an active operation. **No concrete physical backend is enabled:** the tests use an in-memory backend, and the contract cannot enforce behavior inside an arbitrary backend implementation. The pinned package checks do not authenticate third-party dependencies or already executed Python bytecode; the eventual launcher needs a controlled environment and verified imports.

Upstream USB `connect` currently chooses the first matching VID/PID and includes a fallback that re-enumerates by VID/PID. The production binding must claim the selected descriptor directly, stop on disconnect, and avoid silently changing targets. Reusing the generic connector without that change would undermine the gate. [Reviewed USB connector](https://github.com/bkerler/mtkclient/blob/60e07f3b343a4469389f15967626d63e049968d4/mtkclient/Library/Connection/usblib.py#L299)

## Next integration milestone: bind and validate physical USB

Implement a concrete backend for the session gate, including exact-device USB claiming and a reviewed preloader/DA startup path with bounded timeouts. Pin an approved HA100 loader and validate controlled imports in the launcher. Keep writing unavailable in that milestone. The existing reader obtains chip/storage identifiers, capacity and the complete GPT from the session; none of this proves HA100 model identity by itself.

Validate one continuous USB session on the HA100: observe the reboot window, establish the download-agent session, enumerate the layout, read identity partitions twice, and compare hashes. Compare decoded Device ID/MAC values with values recorded from that same remote. Record the upstream commit, loader hash and host USB dependencies with the results. Repeat connection and disconnect tests before adding writes.

Use the repository's actual slot roles when preparing the release: `boot` is p8, `recovery` is p9, and `userdata` is p23. These are reference observations, not permission to assume matching byte offsets on another unit. Keep the rescue kernel independent from the experimental runtime kernel. Test the recovery USB shell with userdata unavailable before proceeding to an end-to-end installation. [Repository partition and recovery procedure](../README.md#recovering-a-bad-boot)
