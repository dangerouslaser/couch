# Direct Wi-Fi stage debugging

The native host can attach to an already running RAM stage without starting the
installer workflow. Attachment does not enroll a device, back up partitions,
admit an installation plan, write boot/storage, reboot, or run a device shell.
An explicitly configured, separately confirmed Linux-only transition can first
write and verify one dedicated debug boot image using retained originals.

Build the host and terminal frontend from the debug branch. Supply a private
configuration outside Git:

```json
{
  "schema": 1,
  "bus": 1,
  "ports": [1],
  "expected_stage": "legacy-status",
  "runtime_root": "/absolute/path/to/prepared/runtime",
  "runtime_receipt_sha256": "INDEPENDENTLY_RECORDED_64_HEX_SHA256"
}
```

Use the existing prepared Python/libusb runtime and its independently recorded
`runtime.json` checksum. The host verifies the receipt and every inventoried
file before executing Python. This path makes no dependency downloads. Use the
same operating-system account as the native installer so its normal state root
and USB lease are shared. Close an existing installer before attachment.

```sh
couch-installer-tui --native-backend /absolute/path/to/couch-installer-host \
  --wifi-debug /absolute/path/to/private/debug.json
```

The configuration cannot be combined with `--config` or `--local-payload`.
Attachment requires exactly one `0e8d:201c` device at the specified physical
bus/port chain and one expected vendor bulk interface. No USB reset,
configuration change, kernel-driver detach, or alternate-device fallback is
performed.

`expected_stage: "legacy-status"` permits only status refresh and close against
an existing installer/probe. A failed legacy stage cannot restart Wi-Fi through
its current protocol. This mode cannot be combined with a boot transition.

`expected_stage: "wifi-debug-v1"` requires exactly one `transition` (below) or
`completed_transition` configuration. A capability-only attachment is refused
before USB access. It also requires opcode 5 to advertise
`wifi_debug: true`, `capabilities: "COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1"`, and
`provisioned: false` and `debug_generation_limit: 8`. Only then can the host request bounded pre-credential
diagnostics (opcode 8) or an explicit startup retry (opcode 9). Both requests
have empty payloads. The worker repeats the identity check for each operation.
The dedicated debug stage must reject provisioning: this host never prompts
for credentials or sends provisioning, plan-binding, or install opcodes.
Opcode 8 must identify `stage_kind: "private-ram-wifi-debug-stage"`,
`capability: "COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1"`, `debug_protocol: 1`, and
`precredential: true`, with a nested status satisfying the same debug identity
gate. Generic status, scan flags and USB descriptor text do not grant debug
capability. Missing or mismatched identity/version fields close the attachment.
The opcode 8 wire payload is capped at 4,608 bytes before reading its body, and
the decoded log at 4,096 UTF-8 bytes before display sanitization. Opcode 9 must
acknowledge with an empty body; it creates the stage's fixed RAM retry marker
idempotently, not an arbitrary command or shell request.
The diagnostic generation must be in 0–8 and advertise the same limit of 8.
At generation 8 or `debug-retry-limit`, the menu offers only refresh/close.
The worker refreshes diagnostics before every retry and refuses to send opcode
9 at the limit. A retry acknowledgement is not proof of a new generation.

Each run creates a new private `wifi-debug-*` session beneath the usual native
installer state root, retaining the same USB lease until it closes. Status,
bounded diagnostics, retry requests and acknowledgements are recorded there.
For attachment alone, existing enrollment, original backups, baseline and
failed-session evidence are not read or modified. A retry acknowledgement means the stage accepted a
request; use its generation and subsequent diagnostics to determine the result.

## Optional no-capture transition (Linux)

This is not the normal bootstrap runner. It never recaptures boot/recovery or
replaces the retained baseline. Add a `transition` object to the configuration
only for a reviewed debug image and an admitted failed native-bootstrap session:

```json
{
  "source": "/absolute/path/to/retained-failed-session",
  "temporary_boot_sha256": "FULL_VERIFIED_CURRENT_TEMPORARY_BOOT_SHA256",
  "original_boot_sha256": "FULL_RETAINED_ORIGINAL_BOOT_SHA256",
  "snapshot_sha256": "FULL_RETAINED_CURRENT_COUCH_SNAPSHOT_SHA256",
  "image": "/absolute/path/to/debug-boot.img",
  "image_sha256": "FULL_REVIEWED_DEBUG_IMAGE_SHA256",
  "metadata": "/absolute/path/to/debug-boot.json",
  "metadata_sha256": "FULL_REVIEWED_DEBUG_METADATA_SHA256"
}
```

All hash placeholders must be replaced by independently recorded 64-character
lowercase hashes. Do not use a temporary installer image as the original-boot
pin. The source session must retain its owner-only journal, all nine original
files, official owner inputs, and pinned DA. Admission verifies the contiguous
failed-bootstrap journal, snapshot and original-file hashes without USB access.
The separately pinned debug metadata must describe a 16 MiB
`private-ram-wifi-debug-stage`, `private_only: true`, `installable: false`, and
an empty `storage_operations` list, with the exact image hash and size. These
pins identify reviewed bytes; metadata alone is not a proof of image behavior.

After offline admission, the frontend defaults to **Cancel**. Confirmation
allows one transition attempt on the configured bus/port chain, waiting up to
180 seconds for the selected `0e8d:2000` preloader. It cannot restart the
currently running legacy stage or force that stage into preloader mode. The
existing reviewed MTK transport may detach/release its selected interfaces;
there is no device reset or alternate-device fallback.

The same USB lease is held throughout transition and later attachment. On the
same connected MTK session, live CID, capacity, partition layout, current
temporary boot and every retained non-boot original hash must match. The
restricted writer rechecks identity and current hashes, writes only `boot`
once, then independently reads back boot and checks retained hashes. Only a
durably recorded successful verification permits one DA watchdog boot request.
An ambiguous write or boot request is never retried automatically. Failure
preserves the original session and new partial receipt for separate recovery.

The host then waits up to 60 seconds for the debug USB stage on the same port,
without returning to backup, enrollment, credentials or final installation.
Future debug runs use the automatically saved private `debug-attach.json` in
the new debug session. This removes `transition` and sets `completed_transition`
to `{ "receipt": "/absolute/path/to/transition/completed.json", "sha256":
"RECORDED_COMPLETED_RECEIPT_SHA256", "inputs": { ... } }`, where `inputs` is
the full original transition configuration (all original/previous/debug and
metadata pins and retained-source paths). The original temporary boot pin is
historical proof, never a request to rewrite that image.

Before every debug attachment, including immediately after a new transition,
an offline validator checks that completed receipt's SHA-256 and its pinned
readback/restart records; re-admits the retained source journal, all original
files, image and metadata; and requires exact CID, capacity, layout, configured
topology, previous/debug boot hashes, boot readback, and retained before/after
checks. Missing, partial, tampered, wrong-topology or legacy receipts fail before
`debug_open`. Keep all referenced files in place; the config is not authority
to replace missing proof with new captures. Legacy status remains receipt-free.
All
new transition receipts live in the new private debug session; retained source
evidence is read-only. The debug capability/status response is a protocol gate,
not cryptographic attestation of firmware identity. The completed receipt proves
the earlier transition, not that nobody has changed the device since then.

Offline checks use mocked USB and the native private event channel; successful
builds/tests do not establish hardware Wi-Fi success.
