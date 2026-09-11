# Direct Wi-Fi stage debugging

The native host can attach to an already running RAM stage without starting the
installer workflow. It does not enroll a device, back up partitions, admit an
installation plan, write boot/storage, reboot, or run a device shell.

Build the host and terminal frontend from the debug branch. Supply a private
configuration outside Git:

```json
{
  "schema": 1,
  "bus": 1,
  "ports": [1],
  "expected_stage": "wifi-debug-v1",
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
its current protocol. Installing/starting a dedicated debug stage is a separate
device-owner operation; this host does not perform that transition.

`expected_stage: "wifi-debug-v1"` requires opcode 5 to advertise
`wifi_debug: true`, `capabilities: "COUCH_PRIVATE_WIFI_DEBUG_STAGE_V1"`, and
`provisioned: false`. Only then can the host request bounded pre-credential
diagnostics (opcode 8) or an explicit startup retry (opcode 9). Both requests
have empty payloads. The worker repeats the identity check for each operation.
The dedicated debug stage must reject provisioning: this host never prompts
for credentials or sends provisioning, plan-binding, or install opcodes.

Each run creates a new private `wifi-debug-*` session beneath the usual native
installer state root, retaining the same USB lease until it closes. Status,
bounded diagnostics, retry requests and acknowledgements are recorded there.
Existing enrollment, original backups, baseline and failed-session evidence
are not read or modified. A retry acknowledgement means the stage accepted a
request; use its generation and subsequent diagnostics to determine the result.

Offline checks use mocked USB and the native private event channel; successful
builds/tests do not establish hardware Wi-Fi success.
