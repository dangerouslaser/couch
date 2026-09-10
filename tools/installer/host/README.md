# Native installer host

This independent Rust workspace begins the host-backend migration with complete
owner-side official input preparation. It verifies the pinned ZIP, bounded
full-OTA transfer lists and Brotli streams, then reads the approved files through
a [read-only Rust ext4 parser](https://docs.rs/ext4-view/0.9.3/ext4_view/).
It executes no subprocess, requires no Python or debugfs, mounts no filesystem
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

This component is not the complete native installer. USB adapter isolation,
session ownership/deadlines, enrollment UX and the write-capable transfer policy
still need migration/integration. Filesystem personalization should move to the
reviewed Linux stage so other hosts do not need native e2fsprogs. The default
public installation gate remains disabled until those flows and recovery are
validated. Native input preparation or native terminal builds alone cannot
establish full macOS/Windows installation support.

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
