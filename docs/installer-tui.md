# Installer terminal and bootstrap

Status: implemented terminal front end and offline simulation; **no approved
public installer release exists**. Neither the TUI nor bootstrap enables hardware
writes. The core installer policy and private developer transport remain separate.

## Terminal flow

`tools/installer/couch_tui.py` displays an ASCII `couch.` wordmark and numbered
menu. It supports Linux with Python 3.10+, without a terminal UI dependency.

1. Review the exact target, release, backup destination, protected identity
   partitions, and ordered write/readback plan.
2. Type the full target ID to confirm; blank or mismatched input cancels.
3. Read progress emitted by the existing policy engine. There are no invented
   percentage bars or optimistic success messages. An error leaves its original
   backups and journal available for recovery.

Default mode reports that physical installation is unavailable. The second menu
option only observes USB descriptors, requiring the existing PyUSB/libusb setup.
An unknown MediaTek device is never labelled as a verified HA100. Simulation is
an explicit `--simulation` option; an unavailable physical adapter never silently
switches to simulation.

```sh
python3 tools/installer/make_demo.py /tmp/couch-terminal-demo
python3 tools/installer/couch_tui.py --simulation \
  --manifest /tmp/couch-terminal-demo/release/manifest.json \
  --device-dir /tmp/couch-terminal-demo/device \
  --identity /tmp/couch-terminal-demo/identity.json \
  --backup-dir /tmp/couch-terminal-demo/originals
```

Use a new empty directory for each demo. The synthetic target confirmation is
`SIMULATED-HA100-001`; these files cannot boot a remote. `--resume` delegates
existing journal verification to the core. See the [captured terminal preview](installer-tui-preview.txt).

`CoreAdapter.plan/apply/observe` is the integration boundary. The terminal only
presents the adapter's plan and streams its output; it does not implement USB
writes or weaken checks. A future approved adapter must supply a verified plan
and execute the reviewed policy, rather than treating terminal confirmation as
permission to skip release, identity, backup, or readback checks.

## Bootstrap contract

`tools/installer/bootstrap.sh` is suitable for a future curl-piped entry point,
but is **not an available public installation command today**. Its approved
version and SHA-256 pins are intentionally empty: it stops before downloading
anything. Publishing requires a separately reviewed installer release and pins.

For a future approved release, it accepts Linux x86_64 only, checks Python/curl,
opens `/dev/tty` for interaction despite piped stdin, and downloads a specific
GitHub release tag over HTTPS. It verifies the pinned SHA-256 before examining
or executing the package; it never selects `latest` or accepts an untrusted URL
or checksum through environment variables.

The package must contain `couch-installer/tools/installer/couch_tui.py` and the
reviewed dependencies/core files in their normal relative layout. Download size
is bounded to 64 MiB, expanded content to 128 MiB/10,000 members. Extraction refuses
absolute/traversal paths, links, device nodes, duplicate names and missing entry
points. Files land in a private temporary directory, removed on exit. This
installer-tool package excludes large OS payload images; payload verification
remains the policy engine's job. Checksums provide integrity relative to reviewed
bootstrap pins, not independent release signing or hardware certification.

Without a terminal, the front end exits with a short explanation. Scripted
planning/testing should use the existing noninteractive core CLI.

## Validation

`python3 -m unittest discover -s tools/installer -p test_tui.py -v`

Tests exercise cancellation, real policy-engine simulation, no physical fallback,
missing terminals, unpublished-release refusal, checksum failure and unsafe
archive members. They never connect to USB or flash hardware.
