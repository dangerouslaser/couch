# Reinstalling from another computer

A retained enrollment can bind a new installer computer to the same remote. Copy
the original private backup directory and its evidence; keep the original copy.
Import is read-only with respect to the remote and old directory. It creates a
new private session copy and never resumes an old installation journal.

The library entry points are `saved_enrollment::import` for native records and
`import_legacy` for retained Python trial evidence. The native orchestrator must
connect these entry points to the reinstall selection; this module alone is not
a standalone installer command.

## Native records

A native source directory contains `enrollment.json`, nine
`bootstrap-NAME.img` originals (five calibration partitions, boot, recovery,
odmdtbo and logo), and the original `event-NNNNN.json` journal. Import requires
all independent original-verification checkpoints before the `OriginalsSaved`
transition and the exact enrollment metadata digest at that boundary. A later
failed installation does not turn these verified original snapshots into a new
capture or authorize resuming its write session.

The importer verifies exact HA100 partition boundaries, canonical CID, complete
calibration inventory, file sizes and hashes, and pinned official Android
boot/odmdtbo prefixes. Vendor Device ID and original MAC records remain private.
The new host receives a `SavedEnrollment`; it does not receive write authority.

## Existing private Python trials

Native records are not retroactively manufactured for earlier trials. Legacy
import requires:

- `bootstrap/baseline.json`, `journal.json` and `backup-receipt.json`;
- `private-image/plan.json` and `originals/journal.json`;
- every original image named by that completed backup receipt;
- the separately trusted stock-baseline manifest and its independently supplied
  expected SHA-256 (for example, from the retained, verified installer package).

A stock-manifest digest read from the same imported directory is not an
independent trust decision. The API requires the expected digest separately and
checks that the old bootstrap journal was bound to precisely that profile.
The profile must identify the reviewed Android HA100 stock baseline; the saved
boot must match its original image digest. Completed bootstrap/readback/cleanup
receipts, canonical plan digest, CID binding, all calibration hashes and every
saved image are checked. Explicit Couch-original records are rejected.

Old records without `original_os` are accepted only through this stock-profile
proof; their absence is never a blanket assumption that arbitrary backups are
Android. Missing vendor Device ID/MAC remains unknown. Missing userdata under an
explicit YOLO policy remains missing; the importer cannot recreate Android apps
or data. A retained odmdtbo hash without its original file is recorded as required
live evidence, not represented as a saved restoration image.

All available originals in the selected legacy receipt, including userdata when
present, are copied through bounded streams and independently hashed again.
Original Python evidence is retained verbatim under descriptive filenames. The
normalized record is named `imported-legacy-baseline.json`; no native enrollment
capture events are invented. Prior source directories and other historical
Android backups stay untouched.

These hashes establish consistency with the owner's retained evidence, not a
hardware signature or an authentication scheme for backups from strangers.

## Binding the live device

`SavedEnrollment::rebind` requires fresh observations from the explicitly selected
physical USB session: MT6580 chip and canonical CID encoding, exact CID, capacity,
full partition map, all five calibration hashes and the retained odmdtbo digest.
Native records bind the saved odmdtbo original; legacy imports bind its digest
from the independently trusted retained stock profile.
Neither a TUI selection nor values copied from the imported record count as live
observations. A mismatch stops before writes.

Rebind returns a `BoundEnrollment` and journals that boundary. The orchestrator
still owns the selected USB port/session, current-state backup and write gates.
Current Couch boot/recovery/userdata snapshots must be classified as Couch;
imported Android originals remain a separate historical set. Never substitute an
old Android boot backup for the current boot image expected by a RAM-stage
transaction. Reverify any retained image again before using it for restoration.

## Without old backups

The current configuration API exposes paired health/configuration access, and
`build.json`/updater status reports runtime version. These are not an enrolled
storage identity or proof of original Android partitions. The current hardware
setup derives a locally administered Wi-Fi address from CID, so its runtime MAC
cannot substitute for the original Android hardware address. The runtime has no
persisted, authenticated Android-original enrollment export.

A separate Couch-only enrollment could use read-only selected-USB capture, known
Couch boot provenance plus reviewed HA100 layout/overlay evidence, and fresh
calibration/current-system backups. It must record `original_os: Couch`, bind the
live hardware, and keep Android restoration unavailable unless genuine retained
Android originals are separately supplied. That admission flow is separate work;
this importer does not silently promote live Couch into an Android baseline.
