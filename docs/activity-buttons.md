# Activity button mappings

In the web UI, open **Activities**, choose an activity, and use **Physical
buttons**. Click a key in the remote illustration, choose a press type, then
select **Device function**, a device, and its function. **Save button mapping**
only saves configuration; reopen the activity on the remote to load changes.

For example, use a Kodi source with D-pad/OK left at their defaults, volume and
mute assigned to a Denon receiver, and power assigned to an LG TV. Targets may
belong to different Couch rooms. Only supported functions are offered. IR
remains unavailable until the built-in blaster can send codes.

- **Activity default** removes an override. **Do nothing** explicitly consumes it.
- Short and long presses are independent. A configured long press fires once
  after 600 ms. Its short action waits for release and never also fires on a hold.
- D-pad, volume and channel keys retain repeat; they cannot have long bindings.
  Repeat events only repeat navigation, volume and channel functions, even when
  a repeat-capable button is assigned to a toggle or power function.
- Overrides apply while the activity screen is open, including its touch sheets.
  Directly opening a device uses normal controls. The touch back arrow always
  exits to Couch. Closing an activity cancels pending holds and queued commands;
  commands already sent to a device cannot be recalled.
- Deleting a mapped device disables its bindings instead of silently redirecting
  them to the source device. Missing references and unsupported commands are
  rejected before saving.

The illustration follows the [HA100 front-panel layout](https://www.sanytron.com/cdn/shop/files/A.2066.png).
The four shortcut buttons above the colored keys were physically captured as
Linux codes 62, 63, 64 and 65 (left to right). Other measured keys are recorded in
`model/couch-model/src/buttons.rs`; the illustration uses accessible HTML buttons.

`Activity.buttons` stores `{button, gesture, action}` entries. `gesture` defaults
to `short`; `action: null` disables the key. Old configurations default to no
bindings. Shared model validation and function catalogs serve both UIs.
Device I/O uses a bounded worker queue, with generation and age checks.

Validation: model/API regression tests, GUI input timing tests, browser
save/reload and mobile-layout checks, and injected evdev presses on the HA100
against isolated Kodi/Denon fixtures. The physical fixture covered short/long
exclusivity, repeat, shortcut and microphone mapping, and cancellation on exit.
