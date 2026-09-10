# Infrared commands on devices

In **Rooms & devices**, open a room and choose **Add IR commands** on an existing
device. A network TV stays the same device: assigned functions use the remote’s
built-in IR transmitter; other functions keep using its connection.

For equipment without a network connection, choose **Manual / infrared** under
**Add to this room**, then enter its name and device type. No separate IR
connection or duplicate device is needed.

1. Choose a brand and model under **Find commands in library or import**.
2. Assign individual commands, or use **Assign matching functions** to match
   common names such as volume up and power toggle.
3. Review **Assigned functions** and remove commands you do not need.
4. Select **Save IR commands** (or **Add device to room** for a new device).
5. Point the remote at the equipment and use **Test** beside a saved command.

Browsing, importing, assigning and saving never transmit. Test buttons stay
disabled while there are unsaved edits. A library match is a candidate; verify
that the equipment responds. Unsupported protocols show an explanation.

**Import your own remote codes** accepts Flipper `.ir` and Couch codeset text.
**Advanced: edit command definitions** supports corrections and custom commands.
Every save creates an immutable codeset snapshot, so editing one device cannot
silently change another device sharing an older codeset. Removing all IR commands
preserves the network connection. Legacy IR devices remain editable.

## Physical controls and activities

Assigned functions override their network equivalents, including physical volume
and activity mappings. A failed IR send reports an error without sending a second
network command. Activity editors show actual assigned IR functions alongside
network functions. Power toggle and discrete power-on/off are distinct: a missing
discrete command never substitutes a toggle.

Selecting an IR-only device opens its controls. Hold Back to return to the room.
IR is one-way: transmission completion does not confirm reception or infer the
equipment’s power, volume or playback state. Existing webOS connection-level power
settings remain supported; configure new command assignments on the room device.

## Regression checks

Build with `tools/build-webui.sh --host`, then run:

```sh
NODE_PATH=build/webui-review/node_modules node tools/tests/ir-library.cjs
```

The browser test uses a real local daemon and isolated configuration. It checks
revision-protected attachment, editing, removal and creation, network preservation,
immutable snapshots, legacy devices, imports, explicit testing and mobile layout.
Catalog and transmission endpoints are mocked; no hardware commands are sent.
