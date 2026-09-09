# Infrared library setup

Add the built-in **Infrared** connection once. In **Rooms & devices**, open a
room and select that connection under **Add to this room**.

1. Name the device and choose its device type.
2. Select a **Brand**, optionally narrow the library device type, then choose a
   **Model / codeset**. Only that brand's models are shown.
3. Assign supported commands to the remote functions you need. Unsupported
   protocols remain visible with the reason they cannot be used.
4. Enter a unique saved codeset ID, review the assigned commands, and select
   **Save and add to room**. Existing IR devices have a **Save IR commands** editor.

The saved ID uses lowercase letters, numbers, hyphens or underscores. Devices
sharing an ID share its commands. Assigning a second source command to the same
function replaces that function's previous code and preserves other functions.
The command text remains editable for custom functions and corrections.

For a remote missing from the bundled library, expand **Import your own remote
codes**, choose Flipper `.ir` or Couch codeset format, paste the file contents,
and preview it before assigning functions. Import preview and saving never send
IR. A library match is a candidate, not proof that the physical device accepts it.
The catalog shows its source and license; inclusion is deliberately limited to
records whose distribution terms have been checked.

LG TVs use the same browser under **Connections → Power control → Choose power
codes from the library**. This picker only offers `power`, `power-on` and
`power-off`; discrete commands never fall back to a power toggle. Save the power
settings separately. Driver presence alone does not prove successful transmission.

## Browser regression

Build the WASM bundle and a host daemon, then run:

```sh
NODE_PATH=build/webui-review/node_modules node tools/tests/ir-library.cjs
```

The test uses a local server and mocked IR/TV endpoints. It checks brand/type
filtering, unsupported commands, function aliases, raw import preservation,
unsafe IDs, revision-protected room creation, LG power assignment and mobile
layout. It does not transmit IR or validate compatibility with physical devices.

## On the remote

Selecting an IR device in a room opens its controls immediately. Physical
navigation, volume, channel, media, color and power buttons use that device's
assigned functions. **Commands** opens a tray containing its assigned supported
commands. Power uses `toggle`; discrete on/off are separate commands. Missing
assignments report an error instead of substituting another code. Hold Back to
return to the room as with the other device screens.

IR is one-way: Couch shows no inferred power, volume or playback state and does
not poll the device. “IR command sent” reports transmitter completion only; it
does not confirm the target received or acted on the command.
