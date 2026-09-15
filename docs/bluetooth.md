# Bluetooth and BLE

Status as of 2026-09-14 (end of day): **the remote is a working BLE HID
remote.** The Bluetooth kernel (`CONFIG_BT` + `CONFIG_BT_HCIVHCI`, unified
kernel `06b21c74`) ships as a boot payload; the toggle under Settings brings
up bridge, dbus, bluetoothd and `couch-bt-hid`; a real TV paired to "Couch
Remote" and took volume keys; and a Bluetooth device routes
the remote's mapped buttons and the one-way TV screen over Bluetooth
([user guide](bluetooth-tv.md)). Idle Bluetooth has no measurable power or
Wi-Fi cost. Pairing is a deliberate two-minute window from Settings or the
web page ([pairing mode](#pairing-mode)); outside it the remote is not
discoverable. Bonds are **per device** (2026-09-15, host-tested, hardware
pending): a device carries its TV's address next to its network connection
and IR commands, chooses a preferred transport, and the on-screen device's
bond is the one link the daemon keeps ([switching](#multi-device-switching-design)). An
in-kernel HCI driver now exists (`hci_stp`, see
[kernel-backports-research.md](kernel-backports-research.md#outcome-2026-09-15-the-in-kernel-driver-on-both-cores)). The sections below are the design record: "what
exists today" describes the starting point, the
[implementation plan](#implementation-plan-branch-bluetooth-rebased-onto-dev-2026-09-14)
and [staging checklist](#staging-checklist) what was done. Kernel-side tasks are mirrored in the kernel tree at
`Documentation/couch/bluetooth.md` on the `bluetooth` branch of
[dangerouslaser/couch-kernel](https://github.com/dangerouslaser/couch-kernel).

## Goal

Let the HA100 act as a Bluetooth Low Energy HID peripheral (keyboard plus
consumer-control remote) towards a TV or streaming box, keep one bond per
target device, and switch which device it is connected to when the user
switches activity. Simultaneous connections are not a goal.

## What exists today

| layer | state | evidence |
|---|---|---|
| radio | MT6580 CONSYS combo chip, Bluetooth 4.0 with LE | MediaTek datasheet; same block that provides Wi-Fi, which works |
| MediaTek transport | built in: `CONFIG_MTK_COMBO_BT=y`, `CONFIG_MTK_BTIF=y` | `kernel/couch-ha100.config` |
| `/dev/stpbt` | created by `connectivity/common/conn_soc/linux/pub/stp_chrdev_bt.c`; opening it powers the BT function on through WMT and pumps raw HCI packets over STP | kernel source |
| Linux Bluetooth core | **off**: `# CONFIG_BT is not set` | `kernel/couch-ha100.config` line 987 |
| HCI device for BlueZ | **none**: `/dev/stpbt` is a character device, not an `hci_dev` | kernel source |
| firmware | the CONSYS ROM patch is already linked into `/` for Wi-Fi by `stage2/hardware-init.sh`; the same WMT patch load covers Bluetooth | stage2 |
| Bluetooth MAC | recorded by the installer as `bluetooth_mac` in the identity backup; stock loads it from NVRAM, our kernel has no NVRAM path | `tools/installer` |
| userland stack | **none**: no BlueZ in the Alpine image, no Bluetooth code in the daemon, GUI or stage2 | repository grep |
| validation | "built in, never exercised" on both from-source and stock | `kernel/README.md` matrix |

Why the core is off: stock Android on MediaTek runs the Bluedroid stack in
userland straight over `/dev/stpbt` through `libbt-vendor`, so the vendor tree
never configured the kernel stack. Nothing in Couch replaces Bluedroid.

## The gap

BlueZ needs an `hci0` registered with the kernel Bluetooth core. The MediaTek
driver only offers a character device. Two ways to close the gap:

1. **Virtual HCI bridge (spike).** Enable `CONFIG_BT` and `CONFIG_BT_HCIVHCI`,
   then run a small daemon that shuttles packets between `/dev/vhci` and
   `/dev/stpbt`. About a hundred lines, no new kernel code, and it answers
   "does the radio work" in a day.
2. **In-kernel STP HCI driver (proper).** A driver that calls
   `mtk_wcn_wmt_func_on(WMTDRV_TYPE_BT)`, registers an `hci_dev`, and maps
   `hdev->send` to `mtk_wcn_stp_send_data(..., BT_TASK_INDX)` and the STP
   receive callback to `hci_recv_frame`. Replaces the daemon once the spike
   proves the radio.

## Kernel constraints to plan around

- Linux 3.18 has no separate LE toggle; enabling `CONFIG_BT` includes LE.
- The management `Add Advertising` command arrived in Linux 4.2, so the BlueZ
  `LEAdvertisingManager1` D-Bus API will not work here. Advertising, including
  directed advertising for device switching, must be driven through raw HCI
  or the older management commands.
- LE Secure Connections landed after 3.18. Pairing falls back to legacy LE
  pairing, which TVs and streaming boxes generally accept. Verify per target.
- The controller comes up as `00:00:46:65:80:01` on every remote unless a
  vendor command programs an address after power-on (it does: see
  [pairing mode](#pairing-mode), "address").
- Whether a newer Bluetooth core could be backported onto this 3.18 base, and
  what it would cost, is surveyed in
  [kernel backports research](kernel-backports-research.md) (`Add Advertising`
  is 4.1, not 4.2; `backports-4.4.2-1` is the last release that carries
  Bluetooth at all).

## Multi-device switching design

BLE HID peripherals connect to one central at a time. The model that works
for multi-device keyboards applies here:

- BlueZ stores one bond per central; nothing extra is needed to keep several.
- The bond lives on the **device**, not the activity: `Device.bluetooth`
  (`{address, name}`) in the model, next to the device's connection and IR
  codeset. Which of those a key tries first is `Device.preferred_transport`
  (`ir` | `ip` | `bluetooth`, absent = infrared, network, Bluetooth), and
  the GUI's executor walks `Device::transport_order`, moving on when a
  transport is unavailable for that press (no IR code, network client cannot
  connect, the device's TV not on the link) and stopping at the first that
  takes it. The Bluetooth-only device of the first cut (`Integration::
  BluetoothTv` / a `bluetooth-tv` connection) is migrated by
  `Config::migrate` into a device with no integration and a bond with an
  empty address; such a bond is sent keys whenever *any* TV is on the link
  and is never activated by address, so it behaves as before until paired
  again.
- The on-screen device holds the link: an activity start activates its main
  device's bond (`Config::bluetooth_link_device`: the source if bonded, else
  the first bonded member), and opening a device screen activates that
  device's. The daemon side of `activate <ADDR>` (drop the other central,
  directed-advertise or whitelist the target) is the daemon's; the GUI sends
  the word once per change through the system service. The daemon persists
  the active bond itself (`/opt/couch/bluetooth-active`) and, with no file
  and exactly one bond, adopts that bond on first start, so nothing on the
  client side re-sends `activate` after a reboot: the GUI sends it when a
  device comes on screen and otherwise trusts the state file's `active`
  line. During a `pair` window the daemon disconnects already-bonded TVs and
  keeps them off the link until the window ends (a 4.0 controller cannot
  advertise with a link up), so `link` is absent for the window and a key
  to the previously active TV falls through to its other transports.
- An activity with two bonded TVs drives the second over its other
  transport. One with none is reported at edit time
  (`Config::bluetooth_conflicts`, shown in the web activity editor) and once
  at start on the remote, not per key.
- Pairing a new TV is per device: undirected connectable advertising for a
  bounded time ([pairing mode](#pairing-mode)) opened *for a device*, then
  the daemon's `peer` line stored on that device by couch-confd.

## Pairing mode

The remote is only pairable and discoverable while the user asks it to be.
`couch-bt-hid` owns the policy; everything else drives it through the key
socket and reads its state file.

- **LE only.** `couch_system::bluetooth` writes `/etc/bluetooth/main.conf`
  (`ControllerMode = le`, `Pairable = false`) before starting bluetoothd. The
  MT6580 is dual-mode, and with the classic side up a TV's Bluetooth menu can
  find "Couch Remote" over BR/EDR first: an LG OLED77G5 did (2026-09-15),
  paired with SSP, searched SDP for a HID record, found only PnP, and dropped
  the link three times over, showing nothing paired on its side, while the
  remote's card said PAIRED (bluetoothd reports the classic bond as
  `Paired`). The trigger was setting `Adapter1.Discoverable`, which turns
  inquiry/page scan on; the daemon no longer touches it and, LE-only, there
  is no classic side to find. `btmgmt info` shows
  `current settings: powered le secure-conn` (no `br/edr`).
- **Address.** `couch_system::bluetooth` programs the controller's public
  address at every bring-up: the Wi-Fi MAC of `wlan0` with the last byte plus
  one (`derive_address`, unit-tested), sent with MediaTek's vendor command
  (`hcitool cmd 0x3f 0x001a <six bytes, little-endian>`, opcode 0xFC1A) while
  hci0 is up, then `hciconfig hci0 down; up`, verified against `hciconfig`,
  and hci0 is put down again so that bluetoothd is the one to power it on:
  the core rejects MGMT's BR/EDR-off on a powered adapter, and an hci0 left
  up after this step came back dual-mode (.158.dev), reopening the classic
  trap above. Before bluetoothd starts, because bluetoothd binds its ATT server to the
  address it saw at init: after a live change every central (an LG and a Mac)
  got no ATT MTU response and hung up. The address survives down/up and the
  toggle's func off/off but not a reboot, hence every bring-up. Why: the
  firmware default is the same on every remote and every boot, and the LG
  keeps per-address state (after one bad round it listed the remote and said
  "unable to connect" without ever sending a CONNECT_REQ); a stable unique
  address makes bonds survive reboots and keeps two remotes apart. Not
  `btmgmt static-addr` (random static): it advertises, but bluetoothd never
  answers ATT on it, verified twice. No Wi-Fi MAC: the default is kept and
  logged.
- **Socket words** (`/tmp/couch-bt-hid.sock`, mode 0600): `pair` sets
  `Pairable` on the adapter, re-registers the advertisement
  with `Discoverable=true` (raw-HCI path: flags byte `0x06`) and opens a
  `PAIR_WINDOW_SECS` (120 s) window; `pair-stop` closes it early; `forget`
  removes every bond without a window and `forget <ADDR>` one bond;
  `activate <ADDR>` makes one bond the active link (the daemon drops any
  other TV and lets only this one connect) and `activate none` lets no TV
  connect. Addresses are uppercase colon-hex (`44:27:45:4E:33:25`), as the
  daemon reports them. Keyboard words `enter`, `escape`,
  `space`, `tab`, `backspace` and `kbd:<hex>` (`kbd:28`, or `kbd:0204` with
  modifiers) send a boot-style report `[mods, 0, key, 0, 0, 0, 0, 0]` then an
  all-zero release on the keyboard report characteristic; the consumer words
  are unchanged. The GUI sends words itself (it links the lib for the paths and
  words); the web daemon goes through `Request::BluetoothPair { action }` on
  the system service, whose `PairAction` is a closed set, so nothing typed on
  a web page reaches the socket.
- **Window edges.** Ends on a peer that is paired-or-bonded *and* has
  subscribed to a report characteristic (`done <name>`), on the timeout
  (`failed timeout`) or on `pair-stop` (`failed cancelled`). At the edge the
  adapter goes back to `Pairable=false` and the
  advertisement is re-registered non-discoverable (flags `0x04`), so a bonded
  TV still reconnects but a phone's Bluetooth menu no longer lists the remote.
  On the managed path the 4.4 core composes the flags itself: with the
  controller LE-only an `hcidump` of a window shows `LE Set Advertising Data`
  with `02 01 06` while the window is open and `02 01 04` before and after,
  the same bytes the raw path writes (with BR/EDR still on the core wrote
  `0x02` in the window, which is what the LG's classic side answered).
  The same non-discoverable advertisement is what the daemon starts with.
- **Peers** come from polling `org.freedesktop.DBus.ObjectManager
  .GetManagedObjects` on `org.bluez` (every 500 ms in the window, every 3 s
  otherwise) for `Device1` `Connected`/`Paired`/`Bonded`/`Name`/`Alias`;
  `StartNotify`/`StopNotify` per characteristic and any report `push()`
  dropped for want of a subscriber are logged in `/tmp/couch-bt-hid.log`,
  because a TV that pairs but never subscribes looks, from outside, exactly
  like keys being ignored.
- **State file** `/tmp/couch-bt-pair.state` (phase line and parser in the
  crate's lib, `PairStatus`): line 1 `<phase>[ <detail>]` with phase
  `idle|pairing|connected|paired|done|failed` (detail = the peer's name, or
  `timeout`/`cancelled`); then, whenever relevant, `peer <ADDR> <name>` on
  `done` (the TV that just bonded: what a device stores), `link <ADDR>
  <name>` while a TV is connected, `active <ADDR>` when a bond has been made
  the active link. `couch_system::ui_settings::LinkStatus` parses those
  lines (and the older `link <name>` without an address) on the readers'
  side; the daemon's lib is expected to grow the same fields. The daemon
  rewrites the file on every poll while a window is open, so readers treat
  a window phase older than `PAIR_STALE_SECS` (window + 15 s) as idle: the
  daemon died. `done` and `failed` are final until the next `pair`; turning
  Bluetooth off removes the file.
- **Requests.** Everything reaches the socket through the system service's
  `Request::BluetoothPair { action, address?, device? }`, so nothing typed on
  a web page becomes a word; `couch_system::bluetooth::word_for` is the whole
  mapping. The CLI forms (`couch-system request '<json>'`):
  `{"BluetoothPair":{"action":"pair"}}`;
  `{"BluetoothPair":{"action":"pair","device":"living-tv"}}` (the window is
  for that device); `{"BluetoothPair":{"action":"stop"}}`;
  `{"BluetoothPair":{"action":"enter"}}`;
  `{"BluetoothPair":{"action":"forget"}}` (every bond);
  `{"BluetoothPair":{"action":"forget","address":"44:27:45:4E:33:25"}}` (one;
  add `"device"` to also clear that device's stored bond);
  `{"BluetoothPair":{"action":"activate","address":"44:27:45:4E:33:25"}}`;
  `{"BluetoothPair":{"action":"activate"}}` (= `activate none`). A bad
  address or device id is refused before any word is formed.
- **Bond request file** `/tmp/couch-bt-bond.request`: written by the system
  service on a `pair` or `forget` that names a device (`<device id> pair` or
  `<device id> unpair`), removed on any of those without one. couch-confd is
  the only writer of the configuration and polls it once a second
  (`Api::tick` → `couch_system::bluetooth::take_pending_bond`): on `done`
  with a `peer` line the bond is stored on the device, on `unpair` the
  device's bond is cleared, and a window that ends without a TV drops the
  request. This is how a window opened on the remote (whose GUI has no write
  path to the configuration) still lands on the device.
- **Readers.** `couch_system::ui_settings::bluetooth_link()` (the bond
  lines), `bluetooth_pairing()` and `bluetooth_peer()` (name only, for the
  rows); the GUI's Settings › Bluetooth panel (row value `ON · <peer>`, a
  second row **Pair with TV** while on, and a modal that owns OK = `enter`
  and Back = `pair-stop`, polled every 250 ms while shown), which every TV
  screen's command/app list reuses for **Pair over Bluetooth** / **Unpair
  Bluetooth** on its own device; the web device card's Bluetooth section and
  Remote page, and `GET /api/remote/device`
  (`bluetooth.pairing {phase, detail, device, bonded {address, name}}`,
  `bluetooth.peer` (name), `bluetooth.link {address, name}`,
  `bluetooth.active`) with `POST /api/remote/bluetooth {"action":
  pair|stop|forget|enter|activate, "address"?, "device"?}` (404 for a device
  the configuration lacks). A device edit or deletion that drops a bond sends
  `forget <ADDR>` for it.
- **Why the keyboard report.** An LG webOS TV completed LE Secure Connections
  pairing and bonding, read the HID service, then showed "press any key on the
  bluetooth keyboard" and dropped the link after a while; every key sent
  during that prompt was a consumer-page report (id 2). The prompt is expected
  to want a keyboard-page report (id 1), which is what OK sends in pairing
  mode. Hardware status is in the staging checklist below.

## Implementation plan (branch `bluetooth`, rebased onto `dev` 2026-09-14)

The first milestone is the spike: prove the radio through Linux's virtual HCI
device with BlueZ on top. Everything below it needs a kernel with `CONFIG_BT`
and `CONFIG_BT_HCIVHCI`. Since [boot image updates](runtime-updates.md#boot-image-updates)
that kernel reaches a remote on the Dev channel as the boot payload of a dev
build, no reinstall; only the BlueZ packages still wait for a new OS image and
are `apk add`ed over SSH on the development remote meanwhile.

**What is on this branch**

- `clients/couch-bt`: `couch-bt-bridge`, the daemon that shuttles H4 packets
  between `/dev/vhci` and `/dev/stpbt`. It reassembles the STP side's byte
  stream into whole frames (vhci requires one packet per write), creates the
  virtual controller explicitly (events written before it exists are
  refused), handles the STP whole-chip reset errnos (88 waits, 99 reopens),
  and can send the set-address vendor command first so `hci0` carries the
  owner's recorded `bluetooth_mac`. The opcode defaults to MediaTek's 0xFC1A
  and is a flag because the HA100 has not confirmed it. Host-testable: the
  framer and the pump are exercised with socket pairs standing in for the two
  devices. Static musl, one dependency (`libc`, for `poll`), like
  `couch-voice`.
- `kernel/couch-ha100.config`: `CONFIG_BT=y`, `CONFIG_BT_HCIVHCI=y`, every
  other Bluetooth profile and transport explicitly off. This changes the
  config hash `kernel/release-pin.json` pins, so a Bluetooth kernel is a new
  candidate through `docs/kernel-release-candidate.md`, not a drop-in. Built
  clean on Ollie 2026-09-14 from the pinned source commit (`ea122a39`, the
  kernel repository's `bluetooth` branch adds only documentation on top of
  it) into `~/couch-kernel/out-bluetooth`; `kernel/release-pin.json` on this
  branch pins that build (zImage `9dd8e84c…`, effective config `1568fe8c…`)
  so `prepare_public_boot.py` exports it. `CONFIG_BT` alone pulled no extra
  symbols in; `olddefconfig` kept the rest of the config byte for byte.
- `tools/provision-alpine.sh`: `bluez` and `bluez-deprecated` join the image
  package list, for `bluetoothd`, `btmgmt`, `hciconfig` and `hcitool`.

**Milestones**

1. *Spike kernel.* Dev build `.140.dev` carries the kernel above as a boot
   image payload: install the runtime, then the boot image the Updates page
   offers after the reboot, then over SSH `apk add bluez bluez-deprecated`,
   `scp` the `couch-bt-bridge-armv7` release asset to `/opt/couch/couch-bt-bridge`
   and run it. Acceptance: `/dev/vhci` and `/dev/stpbt` exist, the bridge
   creates `hci0`, `hciconfig hci0 up`, `btmgmt info` reports LE,
   `hcitool lescan` sees nearby advertisers, and the bridge log shows the
   set-address command answered (or names the opcode that must replace 0xFC1A).
   Done 2026-09-14 (`hci0` up, LE scan sees advertisers). The system service
   starts and stops `couch-bt-bridge` for the **Bluetooth** toggle in Settings
   and on the web UI's remote page (`Request::Bluetooth`, setting `bluetooth=`
   in `settings.conf`, off by default).

   **Wi-Fi and Bluetooth share one combo radio and STP transport.** The first
   spike that took Wi-Fi down (2026-09-14) did so because the bridge was not a
   singleton and started at boot racing Wi-Fi: two bridges on the un-guarded
   `/dev/stpbt` desynced STP, which hit its retry limit and reset the whole
   chip, dropping Wi-Fi. The fixes (all userspace): the bridge takes a
   whole-file lock so only one ever runs; it opens the devices non-blocking and
   backs off on a full transport (ENOSPC) instead of dying and wedging in an
   uninterruptible read; and it is no longer started from `stage2.sh` at boot,
   only by the toggle, by which time Wi-Fi is up. No kernel change was needed;
   stock Android runs both together on this chip. The bridge ships in the
   runtime bundle from .143 (updaters from .142 accept extra top-level
   `couch-*` executables), with a copy in the boot ramdisk as a fallback; the
   service prefers the runtime copy, so bridge fixes are runtime-only updates.
   The kernel still has to go through the candidate checks before promotion.

   Follow-up hardening: if a chip reset ever does fire, re-run DHCP on `wlan0`
   so Wi-Fi re-associates without a reboot.
2. *Power and coexistence.* **Measured 2026-09-14** on the .144 kernel,
   unplugged, screen asleep, over Wi-Fi (fuel gauge `current_now`; the averaged
   field reads 0 on battery). Idle draw was ~110 mA screen-off with Bluetooth
   off, and unchanged with Bluetooth on and the controller idle: the idle radio
   cost is below the gauge's resolution (~5 mA quantization, ~50 mA background
   swing). Wi-Fi throughput (iperf3): ~30 down / ~37 up Mbit/s with BT off; the
   same with BT enabled but the controller down (the toggle's actual state) or
   up-idle (down unchanged, up dips to ~27); it drops to ~13 down / ~23 up only
   during a *continuous* LE scan. Takeaway: enabling Bluetooth costs no
   measurable idle power and does not hurt Wi-Fi; the shared radio time-slices
   Wi-Fi only under sustained BT activity. A HID peripheral advertises and holds
   a low-rate connection rather than scanning, so its expected impact is small;
   confirm once HID lands, and prefer duty-cycled advertising over any scanning.
3. *HID over GATT.* **In progress (`clients/couch-bt-hid`, 2026-09-14).** A
   separate daemon on zbus (bluer was rejected: it needs libdbus, a C lib).
   It registers a HID-over-GATT application (Device Information, Battery, and
   HID with a keyboard + consumer-control report map) via bluetoothd's
   GattManager1, runs a just-works agent, and advertises "Couch Remote" over
   raw HCI (this 3.18 kernel has no MGMT advertising, so LE Set Advertising
   Parameters/Data/Enable go out through hcitool). Confirmed on the .144 remote:
   the app registers, all advertising commands return success, the daemon
   advertises. Remaining: first real TV pairing (the controller address is the
   synthetic 00:00:46:65:80:01 until the set-BD_ADDR vendor command is
   confirmed), sending a key on connect, then wiring startup (dbus + bluetoothd
   + bridge + this daemon behind the toggle; add dbus to the OS image) and a
   dev build. Original notes:
   A second daemon (or the same one grown) that registers the
   HID service with BlueZ over D-Bus, with the keyboard and consumer-control
   report map, and drives advertising through raw HCI since 3.18 has no
   `Add Advertising` management command. First pairing with a real TV; record
   which pairing method each target accepts.
4. *Switching.* **Client side done 2026-09-15** (branch
   `bluetooth-per-device`): one bond per device, the on-screen device's bond
   is activated on activity start and device-screen open, preferred
   transport with fallback, per-device pairing from the web device editor
   and the remote's TV screens, migration of the old connection. The daemon
   side (`activate`, `forget <ADDR>`, the `peer`/`link`/`active` lines) is a
   parallel branch; the two meet on hardware.
5. *Proper driver.* Replace the bridge with an in-kernel `hci_dev` over STP,
   per the kernel repository's task list, once the spike has proven the radio.

## Staging checklist

Kernel (couch-kernel `bluetooth` branch, built on Ollie):

- [x] `CONFIG_BT=y`, `CONFIG_BT_HCIVHCI=y` in `couch-ha100.config`; keep
      `BT_RFCOMM`, `BT_BNEP`, `BT_HIDP` off unless a profile needs them
      (on this branch; the candidate still needs building and re-pinning).
- [x] Build `normal` (Ollie `out-bluetooth`, 2026-09-14; pinned on this branch).
- [x] First boot on the HA100 through the `.140.dev` boot image; `/dev/vhci`
      and `/dev/stpbt` both appear; Wi-Fi, IR, display, keys and keypad wake
      validated on the unified kernel (`06b21c74`, `.144.dev`).
- [ ] Later: in-kernel STP HCI driver replacing the bridge daemon.

Userland (this repo):

- [x] Add `bluez` (and `bluez-deprecated` for `hciconfig`/`hcitool`) to the
      image package list (`tools/provision-alpine.sh`; takes effect in the next image).
- [x] Bridge daemon between `/dev/vhci` and `/dev/stpbt` (`clients/couch-bt`).
- [x] Spike acceptance (2026-09-14): `hciconfig hci0 up`, `hcitool lescan` sees
      advertisers. (Alpine bluez has no `btmgmt`; used `bluetoothctl`/`hciconfig`.)
- [x] Program a stable address at bring-up (2026-09-15): the Wi-Fi MAC plus
      one through vendor command 0xFC1A, before bluetoothd; the identity
      record's `bluetooth_mac` is not needed for this.
- [x] HID-over-GATT peripheral (2026-09-14, `clients/couch-bt-hid`): GATT HID
      service via bluetoothd GattManager1, keyboard + consumer-control report
      map, advertising through bluetoothd's `LEAdvertisingManager1` where the
      kernel's Bluetooth core has it and raw HCI where it does not.
- [x] First pairing with a real TV (2026-09-14): just-works pairing accepted;
      after connect, a consumer volume-up report changed the TV volume. Paired
      with the synthetic controller address 00:00:46:65:80:01 (set-BD_ADDR not
      needed for this TV). Record other targets as they are tried.
- [x] Toggle brings up the whole stack (`couch_system::bluetooth`), publishes
      starting/on/off/error in `/tmp/couch-bt.state`; the GUI row, web page
      and API show it (`.145.dev`, `.147`).
- [x] Key routing: `Provider::BluetoothTv` / `Integration::BluetoothTv`; the
      GUI's mapped-button executor and the one-way TV screen send the
      function id as a datagram to `/tmp/couch-bt-hid.sock` (mode 0600).
- [x] Power validation (2026-09-14, `.144.dev`, unplugged, screen off):
      ~110 mA idle with or without Bluetooth on; Wi-Fi throughput unchanged
      with Bluetooth on and idle, halved only during a continuous LE scan.
- [x] Pairing mode (2026-09-15): `pair`/`pair-stop`/`forget` words, keyboard
      words, the 120 s pairable + discoverable window, non-discoverable
      advertising outside it, `/tmp/couch-bt-pair.state`, the Settings modal,
      the web section and `POST /api/remote/bluetooth`
      ([details](#pairing-mode)). Verified on the dev remote (.155.dev/.156.dev,
      backported core): window opens with Discoverable and Pairable on and the
      advert re-registered discoverable, `pair-stop` → `failed cancelled`,
      timeout → `failed timeout`, both edges back to non-discoverable, `enter`
      with no subscriber logged as a dropped keyboard report, `forget` empties
      the device list, Wi-Fi unaffected. bluetoothd re-applies its main.conf
      `Pairable` default when the adapter finishes starting, after the daemon's
      first Set, so the daemon re-asserts it on every poll (and main.conf now
      says `Pairable = false`). First TV attempt (LG OLED77G5) went over
      classic Bluetooth and failed, see "LE only" above; the LE-only
      controller is the fix (.157.dev). **Validated end to end with the LG on
      .157.dev**: window → connect → bond → TV subscribes → DONE, OK sends
      Enter, advert hidden afterwards, TV off/on reconnects, volume keys work.
- [x] Bond store keyed per device, transport preference and fallback, the
      link following the on-screen device (2026-09-15, host tests only:
      model, couch-system, couch-confd, couch-gui, wasm check). To verify on
      the dev remote with the daemon branch: pair the LG from its device
      card (web) and from its TV screen (remote) and see `bluetooth.address`
      land in config.json within a second of DONE; pair a second TV on a
      second device; start an activity whose source is one and watch
      `active <ADDR>` follow it (and the other TV drop); open the other
      device's screen and watch the link move; a key on the non-linked TV
      goes out over its IR/network transport, or reports "not connected over
      Bluetooth" when it has none; power-on on a device preferring Bluetooth
      goes out by IR; Unpair removes the bond on both sides; a config from
      before (a `bluetooth-tv` connection) comes up migrated and its device
      still takes keys; the Settings row and Remote page's global Pair with
      TV still work and store nothing.
- [ ] Daemon: `activate <ADDR>` / `activate none` / `forget <ADDR>` and the
      `peer` / `link <ADDR> <name>` / `active` lines, the persisted active
      bond and the single-bond adoption (parallel branch
      `bluetooth-bond-slots`); the two branches meet on the dev remote.
- [ ] Re-advertise immediately on disconnect. Done where bluetoothd exports
      `LEAdvertisingManager1` (the backported core, see
      [kernel backports research](kernel-backports-research.md)):
      `couch-bt-hid` registers an `org.bluez.LEAdvertisement1` object at
      `/couch/hid/adv0` instead of running the raw-HCI path, and bluetoothd
      restores the advertisement itself. The 3.18 core keeps the 15 s
      re-enable tick. Host-tested only; not yet run on a remote.

## Open questions

- Does the CONSYS BT function power on cleanly alongside Wi-Fi under the
  built-in WMT driver, or does it need the sleep/wake handling stock uses?
- ~~Which HCI vendor command programs the address on CONSYS_6580?~~ OGF 0x3f
  OCF 0x001a (0xFC1A), six bytes little-endian, effective after a down/up of
  hci0 (2026-09-15).
- Does each target (LG, Apple TV, Android TV, Fire TV) accept a BLE HID
  keyboard with legacy pairing? Record results here as they are tested.
