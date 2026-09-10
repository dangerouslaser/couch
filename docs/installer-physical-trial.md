# Private physical installation trial

The public `couch_install.py install` gate remains closed. The separate
`private_install.py` is a developer validation tool, not the published curl
installer. Its `--allow-private-flash` flag is required even for preflight.

It requires a private candidate manifest (`private_candidate: true`,
`installable: false`), exact full-partition images, pinned MTK source/loader and
same-device board data, a known USB port, and an independently captured runtime
CID/layout/calibration baseline. `--check-only` verifies local inputs without
opening USB. The baseline CID hash must be supplied explicitly; it is not
inferred from a generic MediaTek USB identifier.

After capture it checks both GPT copies and reads every identity partition
against the baseline before constructing a partition writer. The transaction
backs up all five identity partitions and every overwritten partition, then
independently rereads each backup from the device. Only then may it write
recovery, userdata, optional assets, and boot, with complete readback after each.
It does not write bootloaders, calibration, GPT or the recovery boot selector.

The writer accepts only exact manifest files and partition bounds, checks every
image again after backup, and verifies each chunk before transmission. Short USB
writes, unexpected acknowledgements and ambiguous failures poison the session;
it never silently retries. Keep the originals and journal after any failure.

`--resume` requires the matching release, identity and journal and revalidates
completed writes. `--restore` uses the journaled originals, restoring recovery
last. It refuses unexpected partition changes; it is an interrupted-install
recovery path, not permission to overwrite arbitrary changes made after boot.
`--boot-after-install` requests the tested DA exit only after successful readback.
On this remote that exit can power it off: manual side Power and separate
first-boot verification may be required.

## Evidence required before public release

- Full backup, installation and readback on the physical HA100.
- Clean first boot, on-device Wi-Fi onboarding and configuration access.
- Display, buttons, sleep/wake and IR on the installed image.
- Recovery access and an interrupted-install/restore drill with intact identity.
- Publishable firmware inputs, release verification and a versioned package.
- Bootstrap/TUI on each advertised host platform using only documented prerequisites.

Passing transport mocks or preparing a private image does not satisfy these
hardware checks. The public website must retain its experimental installer
status until the checks are recorded.
