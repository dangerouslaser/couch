# The settings menu

`ui/couch-gui/ui/components/settings.slint`, opened by holding the menu key
(KEY_MENU, code 139) on the home screen. Tiered: a root list of sections, and
a panel per section. It owns the D-pad while up, the way the keyboard does -
its own FocusScope, focus on show, focus back to the shell on close.

## Navigation

- Up/down move the cursor, wrapping.
- OK enters a section from the root, or fires a row's action.
- Left/right step a value where a row has one (the Display rows).
- BACK climbs one tier, and closes from the root.

## Sections

**Display** — three stepper rows, all applied live and saved at once:
- Brightness, 10-100% in tens. `system::brightness_level` maps percent to an
  8-bit backlight with a floor (10% -> 30, so the screen never goes dark), and
  the standby dim level follows at a sixth of it.
- Dim after: 15s / 30s / 1m / 2m / 5m.
- Screen off after: 30s / 1m / 2m / 5m / 10m / Never (Never only dims).

**Wi-Fi** — shows the connected SSID and signal, and a "Change network" row.
The change flow (scan, pick, enter the passphrase on the keyboard, reassociate)
is stage B; the row announces the request for now.

**SSH** — a single toggle showing ON/OFF, or "unset" when no key or password
is enrolled (enrolment stays gated on the setup portal's physical-press step;
the toggle only starts and stops sshd). Wiring is stage B.

## Persistence

Brightness and the two timeouts live in `/opt/couch/settings.conf`
(`key=value`, atomic temp-then-rename), read at startup and written on every
change. The saved brightness is applied to the panel on the first frame, not
left at the splash's full.

## The menu key

The hold is timed by the host (`MENU_HOLD_US`, 500ms), not the component: the
keypad reports the key's down/up edges the way it does the microphone's
(`keypad::KEY_MENU`), and `main.rs` opens settings once the hold passes, but
only on the home screen - a modal already up owns the key. The down edge is
recorded before the wake-swallow, so a hold that begins on a dark panel still
opens settings after the wake.

## Testing it by injection

`COUCH_SETTINGS=1` opens the menu at startup so it can be driven without the
menu key. Injected key events MUST be framed with a SYN_REPORT (a 16-byte zero
event) after each, or evdev buffers them and the GUI never reads them; and only
key codes the device declares in its capability bitmap can be injected at all
(the input core drops the rest) - OK is code 353 here, not KEY_ENTER. See
`docs/keyboard.md` on the keybit filtering.
