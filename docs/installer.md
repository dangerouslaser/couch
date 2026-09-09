# Couch USB installer

Status: experimental installer policy engine, simulation CLI, and read-only USB adapter. **This does not yet install Couch on a physical remote.** The `install` command refuses to run. Controlled HA100 testing has captured the selected preloader, synchronized, uploaded the approved download agent into RAM and obtained eMMC/RAM metadata. A subsequent controlled session verified both GPT copies, runtime CID and all five identity partitions, then saved and independently verified their 60 MiB backup. USB teardown returned ENOENT afterward; clean teardown, persistent writes and installer recovery remain unvalidated. Separately, runtime reads of five identity partitions plus boot/recovery have private, independently verified baseline copies.

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

The reviewed suite has 74 tests, including CLI checks that `plan` leaves all fixture files unchanged and `install` refuses before constructing a transport. Adapter tests cover GPT CRCs and disagreement between valid copies, bounded reads, short responses, target confirmation, and independent identity readback. Session-gate tests cover source/loader changes, competing locks, ambiguous device selection and cleanup. Simulation is single-process only; it does not yet use the session gate's lock. Do not treat the simulator as a hardware-ready transaction manager.

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

The gate holds a private POSIX advisory lock during preparation, the wait window and the full session. Cooperating processes must use the same lock directory; it cannot lock out unrelated flashing software. It requires exactly one generic MTK candidate, matches the explicit bus/address/port path, and checks the claimed device again before starting a handshake. Cleanup calls `close(reset=False)` after failures or normal completion. An already disconnected device (`ENODEV`) is not reported as a second failure, and other cleanup errors do not mask the original protocol error. Linux/macOS locking is implemented; Windows locking is not.

The backend contract separates descriptor enumeration, exact-device claiming, and `start_readonly`. Its policy forbids flash writes, reset, security bypass, reconnection and watchdog changes. Loading a DA into RAM remains an active operation. `mtk_usb.ExactUsbBackend` implements the contract experimentally; it is available through the Python API and the explicit read-only capture harness; the installation CLI still refuses hardware writes. Tests use fake USB descriptors and MTK protocol objects. The pin does not authenticate third-party dependencies, which still need a controlled, versioned runtime environment.

Upstream USB `connect` currently chooses the first matching VID/PID and includes a fallback that re-enumerates by VID/PID. The Couch binding bypasses that connector: it claims the selected descriptor directly and disables upstream rediscovery entry points. It stops on errors rather than selecting a new VID/PID match. [Reviewed USB connector](https://github.com/bkerler/mtkclient/blob/60e07f3b343a4469389f15967626d63e049968d4/mtkclient/Library/Connection/usblib.py#L299)

## Concrete USB binding: startup and identity readback validated

`tools/installer/mtk_usb.py` claims the exact enumerated PyUSB device and its CDC bulk endpoints. It records which interfaces it claimed and detached, then releases those interfaces and restores only those drivers during cleanup. It does not reset USB or fall back to setting a new configuration. USB transfer defaults are one second; claim and read operations have ten-second deadlines and startup has a thirty-second deadline. Deadlines require the main thread on Linux/macOS and unwind through cleanup; they are not a separate-process kill guarantee for a defective native USB library.

Imports, verified loader/EMI parsing and upstream object construction now finish before the capture harness starts waiting for USB. Startup uses a strict four-byte preloader handshake (`a0 0a 50 05`, each immediately checked against its complement), with a separate trigger only for the recognized `READY` path described below, and guards the MT6580 chip check, and requires a successful security-configuration response. It refuses protected targets and BootROM sessions. It uploads only the supplied verified loader in stock/unpatched mode, disables reset, rediscovery, security-bypass and flash-write entry points, and does not call the generic `configure_da`. A missing DRAM configuration or any requested reconnect stops startup. Optional `ExactUsbBackend(preloader=..., preloader_sha256=...)` accepts explicitly verified board data for EMI extraction only; those bytes are never written to a partition or used as the downloaded DA. The legacy generic-preloader directory scan is redirected to an empty private directory, and EMI extraction rejects unapproved paths or bytes. No loader or per-unit preloader image is bundled or automatically selected.

The pinned importer compiles verified package source directly and ignores bytecode caches. An offline check successfully imported the real reviewed `MtkConfig` and `Mtk` classes on Ollie using that importer; no USB enumeration, claim or startup was called. Tests cover exact claiming, partial cleanup, one-shot handshake failure, protected/wrong targets, unsupported security queries, deadlines and cached-bytecode avoidance. They do not prove the vendor download agent or hardware behaves as expected.

## Next milestone: validate a read-only hardware session

Validate the candidate loader and explicitly pinned same-device board data on a dedicated read-only test session. Keep writing unavailable in that milestone. The existing reader obtains chip/storage identifiers, capacity and the complete GPT from the session; none of this proves HA100 model identity by itself.

Validate one continuous USB session on the HA100: observe the reboot window, establish the download-agent session, enumerate the layout, read identity partitions twice, and compare hashes. Compare decoded Device ID/MAC values with values recorded from that same remote. Record the upstream commit, loader hash and host USB dependencies with the results. Repeat connection and disconnect tests before adding writes.

Use the repository's actual slot roles when preparing the release: `boot` is p8, `recovery` is p9, and `userdata` is p23. These are reference observations, not permission to assume matching byte offsets on another unit. Keep the rescue kernel independent from the experimental runtime kernel. Test the recovery USB shell with userdata unavailable before proceeding to an end-to-end installation. [Repository partition and recovery procedure](../README.md#recovering-a-bad-boot)

## Loader and board-data inspection

Offline inspection of the reviewed checkout found an MT6580 legacy entry in `mtkclient/Loader/MTK_DA_V5.bin`: hardware version `0xca00`, software version `0`, three regions. Its SHA-256 is `aef234190ccb8145d2e3b8459741e9adb70f2caa8481aa216c1b25152afaca1f`. This identifies a candidate for testing, not a hardware-approved release component or redistribution permission. The `mt6590` bundle also contains an MT6580 entry; its filename alone is not a reason to prefer it.

The development unit's two original preloader backups matched each other, and offline extraction found an EMI version 21 block of 752 bytes. The partition images contain a 2048-byte prefix before the supported preloader header. Keep the images and their per-unit hashes private; an actual test must explicitly pin its own input. Their availability does not prove that another HA100 revision uses identical memory configuration. **The installer never writes preloader partitions.**

Upstream legacy startup otherwise tries to find board data by scanning generic preloader files for a partial DRAM signature. Couch now disables that fallback instead of silently selecting unrelated board data. [Reviewed stage-two configuration](https://github.com/bkerler/mtkclient/blob/60e07f3b343a4469389f15967626d63e049968d4/mtkclient/Library/DA/legacy/dalegacy_lib.py#L300)

## Explicit read-only capture harness

`tools/installer/capture_readonly.py` validates all local inputs before discovery, waits for the selected physical USB bus/port path, and accepts only preloader PID `0e8d:2000`. It ignores Couch's runtime `0e8d:201c` gadget and rejects BootROM, multiple download candidates, or a device appearing on another port. It sends no reboot or power-cycle command; an operator must arrange the intended preloader entry separately.

Supply `--checkout`, `--loader`, `--loader-sha256`, `--preloader`, `--preloader-sha256`, `--backup-dir`, `--bus`, and `--ports` (a dot-separated physical port path). Use `--timeout 120` if needed. Run the same arguments with `--check-only` first to verify local source, image hashes and baseline syntax without importing USB or waiting. The existing Ollie checkout's candidate loader is inside `mtkclient/Loader/MTK_DA_V5.bin`, not a top-level `Loader` directory.

Optional `--baseline` accepts schema 1 with `capacity` in bytes and the complete `partitions` name-to-`offset`/`size` mapping. Optional `cid` is the 32-hex-digit runtime eMMC CID; optional `identity_sha256` must contain all five calibration partition hashes. Baseline syntax is validated before claiming USB. Actual CID/layout can only be compared after DA startup; identity hashes are checked before backups and against the completed readback. Any mismatch stops capture. The report distinguishes layout, CID and calibration-baseline verification, and no private identifiers are printed.

All capture files must be outside the repository. A fresh destination is required; this harness does not overwrite backups, resume a failed capture or automatically reconnect. Run it using the tested MTK virtual environment, an approved loader and the same unit's original board data. No physical capture has been performed by the implementation tests.

## First capture attempt and startup corrections

A controlled hardware attempt observed preloader PID 2000 on the expected port, but the initial handshake failed before any DA upload. The remote subsequently returned to normal Couch operation. Source review found two issues worth correcting before another attempt: preparation occurred inside the short preloader window, and the upstream handshake sent an extra A0 byte before the four-byte exchange. Preparation now happens before waiting; the binding implements the strict exchange directly and reports the failing byte and response. Tests verify the exact transmitted sequence, preparation-before-wait ordering and preservation of protocol errors during cleanup. These were the initial findings; subsequent DA startup succeeded as recorded below, while USB identity readback remains unvalidated.

An offline preparation check then found a separate upstream constructor bug: interface `-1` indexes a dictionary of default USB IDs as though it were a list. Preparation now supplies an explicit placeholder interface without claiming it; startup replaces it with the observed CDC interface. The full real pinned `Mtk` constructor and candidate-loader/board-data parsing subsequently passed offline on Ollie, including EMI version 21 / 752 bytes. No USB enumeration, claim or handshake was used in that check.

A later prepared capture reached the selected preloader but failed with USB overflow before DA upload. The input binding now requests whole USB packets, returns exactly the requested protocol bytes and retains surplus bytes for subsequent reads, including upstream buffered reads. A strict handshake mismatch reports a limited prefix of the unexpected reply/buffer instead of silently flushing it or switching modes. Tests cover coalesced replies, surplus preservation and an unsolicited `READY` packet. Actual packet contents from the failed attempt were not captured, so a specific preamble is not assumed.

Host inspection also found ModemManager probing the same port during the attempts. The controlled test host is applying a temporary udev ignore rule scoped to the remote's physical port and MediaTek preloader/runtime IDs, rather than disabling ModemManager globally. That interference and the packet-size failure are separate findings; successful capture after both corrections still needs validation. The remote returned to runtime after the failed attempt.

## READY banner and the MT6580 download trigger

The next physical attempt exposed an exact `READY` banner, before any DA upload. Published MT6580 preloader source explains this state: `usb_listen` emits that banner and consumes an initial A0 byte before invoking the download handler. The handler then expects a fresh four-byte A0/0A/50/05 exchange. Thus the earlier duplicate-byte hypothesis was incomplete: this particular preloader path needs a trigger plus synchronization, whereas an already active download handler needs only synchronization. No `DOWNLOAD`, META or reset command is required. [MT6580 USB listener](https://github.com/svoboda18/preloader/blob/73d33cc801ec3ab49c886529ca2f19a6b11f2d03/platform/mt6580/src/core/handshake_usb.c#L189), [MT6580 download synchronization](https://github.com/svoboda18/preloader/blob/73d33cc801ec3ab49c886529ca2f19a6b11f2d03/platform/mt6580/src/core/download.c#L679).

The host now recognizes only the complete `READY` token while expecting the initial complement. It sends one additional A0 after a 30 ms gap, accounting for the source's 20 ms listener poll, then strictly validates synchronization. Up to eight already queued exact banners are accepted without sending further triggers. Unknown tokens, excessive banners or wrong complements stop the session. Direct-complement targets retain the four-write exchange. The next physical attempt confirmed this state handling and uploaded both DA stages successfully. No vendor source code was incorporated.


## DA startup and pending readback validation

The fifth controlled attempt completed the READY exchange and uploaded both unpatched DA stages. The loader reported 1 GiB external RAM, 4 MiB eMMC boot areas and user capacity `0x1d2000000`. It then stopped on an empty-or-oversized USB transfer before completing GPT validation. No persistent partition writes were issued. DA mode can remain active after a failed read, so normal OS return must be verified by the operator before continuing.

Bulk transfers can terminate with a zero-length packet. The input adapter now accepts at most eight such empty transfers per logical read, retains all nonempty bytes and keeps the existing operation deadline. Oversized transfers remain fatal with explicit lengths; partition-read errors include offset and requested size without exposing contents. These changes are covered by mock tests and await the next controlled capture. [libusb transfer termination](https://libusb.sourceforge.io/api-1.0/group__libusb__asyncio.html), [packet sizing](https://libusb.sourceforge.io/api-1.0/libusb_packetoverflow.html).

CID representation was checked privately against the same unit's runtime baseline. The pinned upstream parser reads two big-endian 64-bit values, but its diagnostic formatter emits them as little-endian, so printed CID text is not canonical. Recovering the wire bytes and rendering their four little-endian 32-bit registers in big-endian order exactly matched Linux sysfs. This is a hardware-confirmed conversion for the tested MT6580 legacy path, not a general claim about other DAs. The adapter preserves its original wire-based `storage_id` and adds `runtime_cid_sha256` with an explicit encoding label. Baseline comparison accepts only this fixed representation; no alternate permutations are attempted. Synthetic tests cover distinct byte positions and rejection of the raw wire encoding as a runtime CID. Attempt six subsequently validated the complete USB identity backup (below). [Pinned upstream parser/formatter](https://github.com/bkerler/mtkclient/blob/60e07f3b343a4469389f15967626d63e049968d4/mtkclient/Library/DA/legacy/dalegacy_flash_param.py#L130), [Linux CID formatting](https://github.com/torvalds/linux/blob/master/drivers/mmc/core/mmc.c).


## Verified USB identity capture; teardown still unresolved

Attempt six completed both GPT checks, matched the runtime CID and layout, compared all five partition hashes to the private runtime baseline, and wrote independently verified backups totaling 60 MiB. The private report recorded `complete`, `runtime_baseline_verified`, `runtime_cid_verified` and `runtime_identity_verified` as true. Vendor Device ID/MAC decoding remains unvalidated. No persistent partition writes were issued.

The process subsequently returned an error during USB teardown (`ENOENT`). This does not invalidate the verified files, but it does prevent claiming a cleanly closed session or a restored GUI. The operator must confirm normal Couch operation separately. Teardown now identifies each failing release, driver reattachment or resource-disposal operation; unexplained ENOENT remains fatal. New reports distinguish successful readback (`complete`) from successful teardown (`usb_cleanup_verified`).

The CLI now announces GPT completion, each runtime-baseline partition check, backup and independent readback, then the start of teardown. Baseline verification reads 60 MiB before creating the destination; backup and readback add another 120 MiB. Several minutes without a destination was therefore not proof of a stalled session. Each transfer remains limited to 1 MiB and ten seconds, with no automatic reconnect or reset.


### Return to normal Couch after capture six

After the physical power cycle, the unit selected Couch recovery because the BCB
still contained `boot-recovery`. USB serial access confirmed the stock recovery
kernel. Both boot and recovery partition hashes exactly matched the private
pre-capture baseline. Clearing only the first 512 bytes of `para` and rebooting
restored the normal Couch kernel, network access and GUI process; a later check
confirmed the BCB had cleared after the normal startup interval. No boot image
was reflashed. USB teardown itself still needs validation; this manual recovery
must not be presented as automatic installer completion.

Before a controlled reboot from running Couch, allow its 90-second startup
interval to finish and verify the BCB state. Rebooting while it remains armed
can select recovery even when the normal image is intact.


## Capture eight: USB teardown cause and explicit boot experiment

A second full hardware capture again verified the runtime layout, CID and all
five identity backups. Precise diagnostics located ENOENT at kernel-driver
reattachment on interface 0. Ollie's kernel logged `Zero length descriptor
references` and a failed `cdc_acm` probe with `-22`: the running DA does not
provide descriptors that the serial driver accepts. This is not evidence of a
corrupted backup, and blindly ignoring ENOENT would hide the failed restoration.

The experimental `capture_readonly.py --boot-after-capture` option requests the
legacy DA's HOME_SCREEN exit **only after** successful backup and independent
readback. It requires a complete runtime CID/calibration baseline, checks both
ACK bytes, and waits within a ten-second deadline for the original USB address
to disappear before cleanup. It never performs a USB reset, reconnect, or flash
write; an error in capture skips the exit command entirely. The pinned upstream
`finish()` compares an indexed integer to a bytes ACK using identity comparison,
so Couch implements the small strict exchange directly. See the
[pinned legacy finish implementation](https://github.com/bkerler/mtkclient/blob/60e07f3b343a4469389f15967626d63e049968d4/mtkclient/Library/DA/legacy/dalegacy_lib.py#L972)
and [Linux CDC descriptor validation](https://github.com/torvalds/linux/blob/master/drivers/usb/class/cdc-acm.c).

This exit path has regression coverage but is awaiting physical validation.
Reports distinguish requested/acknowledged boot from normal-OS verification;
a protocol ACK alone does not prove a successful Couch boot. The default capture
still sends no exit command. Physical installation remains disabled.


## Restore simulation

`simulate-restore` restores the original files saved by `simulate`, using the
same `--manifest`, `--device-dir`, `--backup-dir`, `--identity`, and
`--confirm-device` arguments. It accepts only the file-backed simulator; there
is no physical restore transport. Retain the matching release bundle as well
as the original backup directory for this experimental command.

Before writing, it verifies the recorded device/layout/identity, every original
backup, current calibration hashes and every untouched or previously verified
target. Only a journaled interrupted write may contain unknown partial bytes.
It restores userdata and optional assets, then boot, and recovery last so the
recovery image remains available through the earlier steps. It never restores
calibration or bootloaders, and never replaces the original backup files.

An interrupted restore requires explicit `--resume`. Once restoration starts,
installation cannot resume from that journal. Tests cover full round trips,
interrupted install and restore, wrong-device rejection, corrupted later
backups, unexpected partition changes, immutable originals and write ordering.
This validates policy and failure handling, not hardware recoverability.
