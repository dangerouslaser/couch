# Linux USB installer stage proposal

Status: design only. This does not change the public installer, current backup
policy, running stock-restoration trial, or physical-device selection rules.

## Why investigate a second stage

The September 10 read-only benchmark of the same 32 MiB `nvdata` partition
produced matching baseline hashes with both readers:

| Reader | Elapsed | Throughput |
| --- | ---: | ---: |
| Existing legacy DA reader | 13.914 s | 2.30 MiB/s |
| Experimental streaming DA reader | 14.020 s | 2.28 MiB/s |

Streaming header setup took 0.0041 s; payload processing took 13.976 s. An
independent Ollie CPU test computed the same volume of additive checksums in
0.067 s and copied it in 0.0028 s. These measurements do not identify the
individual storage, DA, and USB costs, but they rule out repeated command
headers and host checksum computation as useful primary targets. USB was already
operating at 480 Mbit/s; the nominal UART baud setting is not the transfer limit.

Do not adopt the streaming prototype as a speed improvement. Its checksum checks
are useful, but this trial found no throughput benefit.

The 5,905,055,744-byte userdata partition takes roughly 41 minutes per full pass
at 2.30 MiB/s. Complete backup, independent backup verification, pre-write checks,
writing, and independent readback can require several passes. Reducing data sent
over USB could matter as much as increasing raw bandwidth.

## Proposed components

**Host CLI:** retain pinned preloader/DA startup, exact physical USB-port
selection, verified board data, and independent baseline checks. After booting
the installer stage, use a Rust host client with libusb on Linux and macOS.
Windows would need a tested WinUSB binding and signed/distributable driver setup;
do not claim Windows support from libusb availability alone. No existing SSH,
Wi-Fi, Android application, or configured Couch installation is required.

**Installer image:** a small Linux kernel and self-contained initramfs containing
a statically linked Rust transfer agent. Its root and working files live in RAM.
It must not mount userdata or start the normal Couch services. Logs stay in RAM
and are copied to the host. Remove the current recovery's persistent diagnostic
markers and dependency on `/mnt/alpine/opt/couch/stage2.sh` from this separate
image. Keep a diagnostic ACM interface if it can coexist without exposing an
unrestricted production shell.

**USB transport:** use FunctionFS with explicit bulk endpoints and a framed,
versioned protocol. The checked kernel configuration already enables
`CONFIG_USB_G_ANDROID`, `CONFIG_USB_F_FS`, and `CONFIG_USB_MTK_HDRC`; the Android
gadget implementation exposes the `ffs` function. That is evidence of a feasible
implementation path, not proof of working descriptors or high throughput in a
new rescue image. Verify the exact kernel embedded in the installer image. Do
not assume configfs, CDC-NCM, or host-independent network gadget support.

The agent accepts partition names from a verified allowlist, never arbitrary
block offsets, shell commands, or filesystem paths. Bind the new USB session to
the expected eMMC identity, observed GPT, installation nonce, manifest hash, and
host journal. Re-enumeration into Linux is an explicit protocol transition; it
must not silently select another connected remote. Authenticate the transition
and manifest before accepting writes. USB VID/PID and an unauthenticated echoed
nonce are insufficient device identity proofs.

## Bootstrap and original backups

A Linux stage cannot currently be assumed to boot directly from the DA's RAM.
The pinned legacy implementation's `boot_to` method is unimplemented. Writing a
recovery image also does not itself instruct the bootloader to select recovery.
An automatic RAM boot would need separate research and validation.

A feasible alternative is a reviewed temporary boot-slot bootstrap:

1. Verify model, CID, both GPT copies, loader provenance, and the user's selected
   device. Independently back up and verify all identity/calibration partitions,
   original boot, and original recovery on the host. Create a private durable
   journal before any persistent write.
2. Record authorization for the specific temporary boot image, then write and
   independently verify only the boot partition through the existing strict DA
   writer. Preserve recovery and every bootloader partition. Establish how the
   supported hardware exits DA mode and starts this image; the current DA exit
   requires a physical Power press on the tested unit.
3. The Linux stage reports its own image/version, CID, GPT, and nonce. Compare
   them with the first-stage journal before continuing. Keep userdata unmounted.
4. Read the complete original userdata partition into a lossless host backup.
   Verify the backup against an independent second storage read. Finish every
   other required original-partition backup and record durable completion.
5. Only after those checks, write the approved OS targets, verify them, and
   install the final boot image last. Verify identity and retained partitions
   again. First normal boot and recovery behavior remain separate acceptance
   checks.

This deliberately changes the current transaction rule that **all affected
partitions are backed up before any write**. The temporary boot write occurs
before the large userdata backup. It still preserves every partition before
modifying that partition, but it needs explicit policy review, failure analysis,
and a separately authorized implementation. Do not silently apply it to the
existing installer or generalize the user's disposable-test-data waiver.

## Transfer and verification

Start with bounded chunks, sequence numbers, exact lengths, per-chunk hashes, and
manifest-defined total lengths. Keep memory bounded; a 1 GiB remote must never
buffer a whole userdata image. Test a small queue of in-flight bulk transfers
before increasing queue depth.

Use optional lossless compression or explicit zero-block records. Hash the
uncompressed bytes and enforce decompressed-size limits. Encrypted Android
userdata may compress poorly, so complete original-backup preservation must not
depend on a compression ratio. Never omit unallocated filesystem blocks from a
bit-exact original backup.

After writing, flush storage and perform an independent read, bypassing stale
page-cache contents with a validated aligned direct-I/O path. The host verifies
returned digests against the approved uncompressed manifest. Device-side hashing
avoids transferring full readback data over USB, but moves part of the trust
boundary into the installer agent: pin its image and validate its hashing and
readback implementation against host-received bytes. A transport acknowledgment
or hash of the input buffer is not storage readback.

If using sparse installation images, explicitly define zero ranges and verify
the resulting full partition. Skipping zero chunks over old data cannot produce
the promised raw image. Filesystem creation plus file extraction is another
possible format, but would require its own manifest and verification contract.

## Disconnects, interruption, and rollback

The host journal is authoritative and durable; record `writing` before each
write and `verified` only after readback. The agent keeps at most a bounded
volatile operation state. On USB loss, stop accepting new writes and leave the
RAM stage available for a fresh authenticated handshake when possible.

A fresh process must revalidate image hashes, physical identity, GPT, journal,
and affected partition contents. Do not retransmit an ambiguous chunk or restart
an entire partition automatically. Resume is an explicit action against a
reconciled transaction. Never accept only the remote's remembered offset as
proof of completed work.

If interrupted before userdata modification, restoring the independently saved
original boot should recover the previous OS, provided all original partitions
remain unchanged. After userdata modification, restore the verified complete
userdata backup and other changed originals before restoring original boot last.
If Linux cannot start, the pinned DA path must remain available for this slower
rollback. Failure of the temporary image must not trigger unreviewed writes to
GPT, BCB, preloader, or `lk`.

## Benchmark and acceptance plan

Run benchmarks only in a fresh, explicitly selected session after the current
trial has finished. No profiler or competing USB client should attach to an
active installer session.

1. Validate the installer kernel and RAM-only operation without storage writes.
   Verify FunctionFS enumeration and authenticated host/device binding.
2. Measure a fixed RAM buffer over bulk USB in both directions. Record host and
   device CPU time, transfer sizes, queue depth, and USB speed. This isolates
   transport from eMMC.
3. Measure aligned uncached reads of a known small partition locally in Linux,
   then stream that same partition to the host and compare its full hash with
   the existing baseline. This distinguishes storage and transport limits.
4. Compare uncompressed and compressed reads using real data and zero-rich
   fixtures. Report both wire throughput and uncompressed throughput; neither is
   a substitute for elapsed backup time.
5. Test storage readback/cache behavior and write recovery on a disposable
   laboratory target before authorizing any production-device writes. Exercise
   USB loss, short transfers, corrupt frames, full host disk, process termination,
   and power loss at journal boundaries.
6. Report end-to-end times for original backup, independent verification,
   installation, and rollback. Retain the existing DA fallback even if the Linux
   stage proves faster.

No Linux-stage bandwidth estimate is yet verified. Proceed only if measurement
shows a worthwhile end-to-end improvement while preserving recovery guarantees.
