# Couch installer

The installer is being prepared for public testing. The private Wi-Fi flow can
install and restore this development remote; the complete desktop package still
needs fresh-device and platform acceptance before publication. A native terminal
binary by itself is not a complete installer. A public curl command will be
added here when the verified release is available.

## Installation flow

1. Start with Android, enable USB debugging, connect the remote by USB, and
   authorize the computer when Android prompts.
2. Select the remote. The installer binds its physical USB connection and storage
   identity, verifies the supported partition layout, and saves the originals.
3. Choose a scanned Wi-Fi network or enter an SSID manually, then enter its
   password. The installer uses Wi-Fi for the large backups and OS transfer.
4. Keep USB connected while installation and independent readback complete.
   Progress distinguishes transfer from verification. If automatic startup fails
   and the screen remains off, hold the side Power button until it turns on,
   then release it.
5. Verify the Couch welcome screen and network connection. The selected network
   is carried into the installed OS, including hidden networks.

The default preserves Android userdata. **YOLO** skips only that large backup;
original boot/recovery, calibration preservation and write verification remain
required. Without a userdata backup, restoring the previous Android data is not
available. OS installation writes a compact filesystem image and grows it on the
remote; it does not transfer a full partition of unused zeros.

## Implementation

The [Ratatui terminal](../tools/installer/tui/README.md) provides the host UI.
The [native Rust host](../tools/installer/host/README.md) implements verified
input preparation, private sessions, fresh Android enrollment, retained-enrollment
reinstallation, USB supervision,
Wi-Fi selection and the complete authenticated installation transaction. Python
is retained only for the reviewed MediaTek transport and fixed USB setup bridge;
its runtime is downloaded and verified by Rust. The Linux stage's storage writer
and verifier are Rust. Hardware acceptance of this newly connected native path
remains separate from passing fixtures and platform builds.

[Owner-side inputs](installer-public-inputs.md) are extracted from the pinned
official firmware archive; device originals come from the actual remote.
Credentials, device calibration and original images belong in private directories
outside Git. Never write `preloader_*` or `lk`.

An interrupted operation retains its originals and journal. Do not treat a
completed transfer as a verified installation or automatically retry an ambiguous
write. Follow the saved restore instructions and
[device recovery guide](device-recovery.md).

## Developer references

- [Wi-Fi stage and transaction](installer-linux-usb-stage.md)
- [Wire protocol](../tools/installer/linux_stage/PROTOCOL.md)
- [Storage policy and direct readback](installer-storage-policy.md)
- [Wi-Fi scan and entry flow](installer-wifi-wizard.md)
- [Release discovery](installer-release-discovery.md)
- [Corresponding source and notices](corresponding-source.md)

Physical startup, full installation, Android restoration and update rollback
must be recorded separately from host fixtures and cross-platform compilation.
Private trial artifacts and session history are kept outside published releases.

Worker startup failures report an allowlisted exception category, numeric USB error codes, and a reviewed adapter source filename/line. Exception messages, paths, locals, and device data are excluded. A failure stops the worker; the diagnostic does not authorize an automatic retry or restore.

On Linux, a newly enumerated preloader node may appear before udev applies its existing permissions. The adapter allows up to one second for access to that exact selected device, retrying only libusb access-denied errors before any handshake. Persistent access denial stops installation: check that the installer user's effective groups include the group granted by the device's udev rule. Do not run the installer as root or broaden access to unrelated USB devices.

For a local acceptance build or an already-downloaded OS archive, pass `--local-payload /absolute/path/package.tar.gz` alongside `--config installer.json` and `--native-backend`. The installer copies the regular file into its private session with bounded progress and applies the same pinned size, SHA-256, manifest, and member checks as downloaded inputs. This option does not make dependency/official owner-input preparation offline and does not weaken HTTPS downloads.
