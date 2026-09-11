# Native installer host

The `frontend` module communicates directly with the Ratatui terminal over its
private inherited socket (Unix) or pipes (Windows). Prompts use increasing IDs;
invalid, stale, cancelled, truncated or oversized replies stop the channel.
Input buffers are zeroized and secret replies are never echoed as display state.
The `--ui-smoke` command is an explicit device-free interface fixture. The native installation orchestrator uses the same channel for the complete
fresh-Android and retained-enrollment reinstall flows described below.

The independent `prepare-official` command performs complete
owner-side official input preparation. It verifies the pinned ZIP, bounded
full-OTA transfer lists and Brotli streams, then reads the approved files through
a [read-only Rust ext4 parser](https://docs.rs/ext4-view/0.9.3/ext4_view/).
That input-preparation command executes no subprocess, requires no Python or debugfs, mounts no filesystem
and opens no USB/device handles. It uses ordinary private scratch files.

```sh
cargo build --release --locked
./target/release/couch-installer-host prepare-official \
  /path/to/official.zip /path/outside-git/new-private-inputs
```

The output matches the owner-input and private-vendor receipts consumed by the
existing installer prerequisites. Exactly four bootstrap members and 33 runtime
files are accepted using compile-time pins. Preloader is an EMI input only;
there is no preloader writer. Output is published only after all hashes verify,
and remains private/noninstallable with no redistribution authorization.
Distribution firmware is not an original-device backup.

The Windows binary is `couch-installer-host.exe`; input preparation uses the
same native Rust pipeline. CI runs portable fixtures on Linux, macOS and Windows.
The manual workflow option additionally downloads the pinned official OTA and
executes the real pipeline on all three hosts without uploading vendor outputs.
The original vendor URL uses HTTP; the reviewed repository SHA-256 pin checks
its bytes before parsing, but does not create an independent vendor signature.

The native orchestrator now connects these inputs to supervised USB startup,
Wi-Fi setup, backups and the Linux-stage writer. Filesystem expansion and
personalization run on the remote, so host e2fsprogs are unnecessary. Public
release readiness still requires a verified release payload and separate physical
acceptance; native builds and preparation tests alone do not certify installation.

## Session journals

`SessionGuard::create(new_directory)` starts a single attempt under an existing
private state directory outside Git. Unix parents must be owned by the effective
user and mode0700; macOS parents with extended ACLs are rejected. Windows requires
a filesystem with persistent ACLs, validates owner/access grants, creates a
protected user/System DACL and holds directory handles against replacement.
The caller should use its private per-user application state directory.

`transition(Phase, evidence)` enforces the reviewed forward sequence;
`checkpoint(evidence)` records per-partition hashes without advancing that phase.
Persist a checkpoint before sending a corresponding device acknowledgement.
Evidence is bounded to64KiB per event and must contain hashes/receipt references,
not credentials. Events are immutable, atomically published and flushed; Windows
uses write-through rename because it has no Unix directory-fsync contract.
Actual power-loss durability on host storage remains a physical validation.

A persistence failure disables that guard. Existing directories, including
interrupted writes, cannot be opened as new sessions. Drop only releases the
process lock and preserves originals/journals. Explicit recovery is separate.
The session lock excludes another owner of that run; the USB adapter must also
enforce device-wide exclusion across different run directories. These primitives
do not open USB or authorize writes.

The native library also contains Linux-stage transport primitives. `stage_tls`
connects only to an explicit selected address using TLS 1.3 and the certificate
provisioned over USB, checks the exact peer certificate before sending the
session token, and applies absolute deadlines to socket I/O. It uses
[Rustls's explicit provider and trust-store configuration](https://docs.rs/rustls/0.23.44/rustls/client/struct.ClientConfig.html),
without system roots, fallback trust or automatic reconnect.

`stage` bounds JSON and raw/zlib frames and validates monotonic verification
progress. `stage_files` independently reads back saved host backups, compares the
device's readback hash and identity pin, and rechecks each image chunk immediately
before sending. Saving a backup returns its hash without acknowledging it: the
orchestrator must persist a journal checkpoint before permitting the next device
phase. These APIs have loopback/file fixtures on Linux and macOS; the host CI
matrix also runs them on Windows.

`transaction` drives a previously admitted USB-bound plan through original boot
copy/readback, complete selected backups, recovery/userdata/optional image writes
and final boot. Every acknowledgement follows a durable `SessionGuard` checkpoint;
original files live directly beside the journal so its directory synchronization
also commits their entries. Image contents are reverified after backups and again
per chunk. Explicit YOLO omits only userdata backup. Failed expansion or journal
publication prevents the final boot acknowledgement. Completion leaves the restart
decision to the caller; there is no automatic reconnect, restore or retry.

The integrated orchestrator below connects these components. Direct library
callers must still verify the release, enrollment evidence, selected device and
USB plan binding before using the transaction API; it deliberately does not
infer admission from a caller-supplied plan alone.

## Native installation orchestration

`couch-installer-tui --native-backend /path/to/couch-installer-host --config
/path/to/installer.json` now enters the native installation flow. Use the
release-specific launcher to verify these files together; an arbitrary local
configuration is not a trusted release. Selecting Cancel happens before
configuration loading, downloads, private-session creation or USB access.

Rust owns dependency preparation, owner-side OTA extraction, fixed public payload
admission, session/USB locks, Android enrollment, progress, Wi-Fi selection and
TLS installation. The embedded Python worker retains only the reviewed MTK
handshake and a fixed USB setup protocol. It uses the independently verified
owner-local Python/MTK/libusb bundle, never a system-library fallback. Pipe reads,
writes and nested USB operations have finite deadlines; cancellation kills the
worker. Windows uses a kill-on-close job, Linux a parent-death signal, and macOS
inherits the TUI's process group.

Fresh enrollment requires authorized Android ADB, canonical storage CID, exact
physical USB selection, the pinned HA100 partition boundaries and matching stock
boot/device-tree prefixes or the reviewed retained-stock full-image pair. Original calibration, boot, recovery, device tree and
logo are read, saved, independently read back and journaled before the sole USB
boot write. Device ID and unavailable MAC addresses are entered from Android;
Android ID or serial is never substituted for the vendor Device ID. Free space
for the selected backups is checked before bootstrap. No preloader or LK write
operation exists in the worker.

Reinstallation imports a [saved enrollment](../../../docs/installer-saved-enrollment.md)
into a new private session. The selected Couch USB port is queried using a
nonce-framed fixed CID command; only a matching retained CID permits one fixed
USB reboot. An unavailable serial interface offers manual restart, while a CID
mismatch or ambiguous reboot stops without retry. Download-mode CID, full layout,
calibration and retained device-tree identity are checked again before writing.
Current Couch originals are saved separately and marked Couch; imported Android
originals are preserved for Android recovery and never replaced by Couch backups.

The public payload contains only `manifest.json`, `userdata.ext4`,
`installer.cpio.gz`, `boot.cpio.gz`, `recovery.cpio.gz`, `zImage` and `logo.bgra`.
The manifest uses schema 1, kind `couch-public-os-inputs`, release `version`,
`source_commit`, and a `files` map of exact size/SHA-256 pairs. Owner vendor data,
stock kernel/header material and retained logo frames are assembled locally.
Non-files, unknown paths, duplicate entries, changed hashes and trailing payload
data are rejected. No private device baseline is a public input.

After USB-bound Wi-Fi credentials and TLS identity are provisioned, Rust performs
all requested backups, transfers the 33 pinned owner vendor files, and writes the
compact filesystem. The stage expands it, installs and reads back vendor/Wi-Fi
files, checks calibration, and commits normal boot last. The final restart is an
explicit choice after verification. YOLO omits only the userdata backup.

These source paths and fixtures do not certify a physical installation on every
host. Windows additionally requires a usable driver binding for the selected
MTK download interface and installer vendor interface. The installer reports a
bounded claim failure; it does not replace drivers for other USB devices. See
[libusb's Windows driver documentation](https://github.com/libusb/libusb/wiki/Windows#driver-installation).
Fresh Android enrollment, OS startup, restoration and reinstall from another
computer need separate physical acceptance records before a public-ready claim.

### Offline owner-image assembly acceptance

Before a release reaches USB testing, exercise the actual public archive with
verified owner inputs through all three native RAM and boot-image assembly paths:

```sh
cargo run --locked --example verify_owner_images -- \
  /private/installer.json /private/public-inputs.tar.gz \
  /private/prepared-owner-inputs /private/new-assembly-check
```

The example verifies the public archive against its descriptor, checks the
compiled owner-file pins, and assembles normal boot, recovery, and installer
images using the same APIs as the installer. It writes private image readbacks
and an `assembly.json` receipt. It performs no network requests, USB operations,
or device writes. Keep its owner-derived images private. Successful public tar
hash admission alone does not establish boot-image assembly compatibility.
