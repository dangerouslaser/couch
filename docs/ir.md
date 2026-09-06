# Infrared

Point the remote at a television and press a button; the blaster on the top
edge sends the code the television answers to. This is the transmit side only:
the HA100 has an IR **transmitter**, not a receiver, so it can drive other gear
but cannot learn a code by watching a remote. Learning would need the receiver
the board does not have.

`clients/couch-ir/` builds one binary, `couch-ir`. It encodes a command in a
consumer-IR protocol, turns that into the exact bytes MediaTek's blaster driver
expects, and writes them to `/dev/irtx`. Everything except the final write is
pure arithmetic and is proven by `cargo test`; the write itself is one small
module, and the hardware confirmation it still needs is the subject of the last
two sections.

## What the hardware is

The blaster is a MediaTek `mt_irtx` character device:

| | |
|---|---|
| node | `/dev/irtx`, mode 0660 root:root |
| major | 243 |
| sysfs | `/sys/class/mt_irtx/irtx` — exposes only `dev` and `uevent` |
| driver name | `mt_irtx` |
| device-tree node | `mt_irtx_pwm`, properties `pwm_ch = 0`, `pwm_data_invert = 0` |

Two things follow from the sysfs class carrying no writable attribute. Transmit
is **not** a sysfs poke; it is a `write()` to the character device. And the
device-tree properties are the fingerprint that identifies *which* `mt_irtx`
driver this is — there are two families, and they take different bytes.

## The ABI, and where it comes from

`mt_irtx` is not in mainline Linux, and the two trees this project's README
cites for the MT6580 — [`parthibx24/k80`](https://github.com/parthibx24/android_kernel_mediatek_k80)
and [`LCM-MTK/android_kernel_mediatek_mt6580`](https://github.com/LCM-MTK/android_kernel_mediatek_mt6580)
— carry only a `Kconfig` and `Makefile` under `drivers/misc/mediatek/irtx/`,
with the driver body pulled from a per-platform subdir that is absent. The body
is identical across every MediaTek tree that does ship it, so it was read from
sibling trees and cross-checked against MediaTek's own userspace HAL. Three
sources, all consulted:

1. **The kernel driver that matches this device**, `mt_irtx_pwm.c` — reads
   exactly `pwm_ch` and `pwm_data_invert`, compatible `mediatek,irtx-pwm`,
   creates `/dev/irtx`:
   [SoCXin/MT6737 `.../irtx/mt6735/mt_irtx_pwm.c` @ 89eaf42](https://github.com/SoCXin/MT6737/blob/89eaf42ad72e6fcde9b1179c7be30e05968fe63c/linux/kernel/drivers/misc/mediatek/irtx/mt6735/mt_irtx_pwm.c)
   and its header
   [`mt_irtx.h`](https://github.com/SoCXin/MT6737/blob/89eaf42ad72e6fcde9b1179c7be30e05968fe63c/linux/kernel/drivers/misc/mediatek/irtx/mt6735/mt_irtx.h).
2. **MediaTek's Android HAL**, which is the userspace that opens `/dev/irtx` and
   builds the write buffer — the authority on the buffer format:
   [StyxProject/vendor_mediatek_opensource `.../consumerir/src/consumerir.c` @ 856297f](https://github.com/StyxProject/vendor_mediatek_opensource/blob/856297f9007ca310ae33d8d54881789a15135926/hardware/consumerir/src/consumerir.c).
3. **The older register-style MT6580 driver**, compatible `mediatek,IRTX`,
   kept for contrast (it is *not* this device — it reads `gpio_pwm`/`major`, not
   `pwm_data_invert`):
   [rock12/ALPS...CENON6580... `.../irtx/mt_irtx.c` @ 7d123b2](https://github.com/rock12/ALPS.L1.MP6.V2.19_CENON6580_WE_1_L_KERNEL/blob/7d123b24605a56b52739cba2a748098cadcb04b7/drivers/misc/mediatek/irtx/mt_irtx.c).

### The ioctls

From the vendor header and HAL. The argument is always a single `unsigned int`
passed by pointer, which is 4 bytes on both the armv7 device and a 64-bit host,
so the ioctl numbers are the same everywhere (unlike ALSA's, which move with
`sizeof(long)`). Transcribed and asserted in `src/abi.rs`:

| name | definition | number | on this driver |
|---|---|---|---|
| `IRTX_IOC_SET_CARRIER_FREQ` | `_IOW('R', 0, u32)` | `0x40045200` | accepted, **ignored** |
| `IRTX_IOC_GET_SOLUTTION_TYPE` [sic] | `_IOR('R', 1, u32)` | `0x80045201` | returns `1` |
| `IRTX_IOC_SET_DUTY_CYCLE` | `_IOW('R', 2, u32)` | `0x40045202` | not implemented |
| `IRTX_IOC_SET_IRTX_LED_EN` | `_IOW('R', 10, u32)` | `0x4004520A` | GPIO enable, unused here |

The transmit itself is **`write()`, not an ioctl.** The ioctls only configure.

### The write buffer

`dev_char_write` DMA-copies the buffer, optionally inverts every byte (if the
device-tree `pwm_data_invert` is set — ours is 0, so no inversion), and clocks
it into the PWM block in memory mode. So the buffer is a **PWM sample
bit-stream**, played LSB-of-word-0 first, little-endian words. It is built in
userspace by the HAL's `signals_generate`, and `src/pwm.rs` is a faithful port
of that function — the point being that the bytes we hand the kernel are the
bytes the stock firmware handed it, whatever the PWM hardware then does with
them.

The generation, for a mark/space table in microseconds and a carrier in Hz:

* A **tick** is `round(26 MHz / (3·carrier))` clocks ≈ `1e6/26e6·h_l_period`
  microseconds — about 8.77 µs at 38 kHz. Each bit in the buffer is one tick.
* A **carrier period** is a whole number of ticks — three, for every carrier we
  use. Each mark/space duration is rounded to a whole number of carrier periods.
* The last `u32` of the buffer is **not waveform.** It is the total transmit
  time in microseconds; the driver plays `word_count − 1` words, so this trailer
  word is never clocked out — its presence is what makes that length arithmetic
  come out to the waveform length. `pwm.rs` reproduces both its presence and its
  value.

### Two "solutions", and which this device uses

`IRTX_IOC_GET_SOLUTTION_TYPE` tells userspace which of two encodings the driver
wants:

* **Type 0, "IRTX+PWM":** the buffer is a plain on/off gate — all ones through a
  mark, all zeros through a space — and a hardware carrier generator adds the
  38/40 kHz. This is the HAL's fallback when the ioctl is unrecognised.
* **Type 1, "PWM-only":** there is no carrier generator, so the carrier is
  **baked into the buffer** — during a mark, one tick on and two off per
  three-tick period (~33 % duty); during a space, nothing.

`mt_irtx_pwm.c` returns **1**, and its probe never writes the hardware IR
registers (`IRTXCFG`, `IRTX_L0H`, …) at all — it only ever uses the PWM. So this
device is **type 1, PWM-only, carrier baked in.** `couch-ir` does not assume
this: on a real send it *asks* the driver (`tx.rs::transmit`) and encodes to
whatever it answers, exactly as the HAL does. Both encodings are implemented and
unit-tested, because the query is one ioctl and getting it wrong is silent — a
type-0 buffer sent to type-1 hardware is an unmodulated blob no receiver
demodulates, and a type-1 buffer to type-0 hardware is carrier-on-carrier.

### Confidence

**High** that this is the right driver family and the right write format: the
device-tree properties (`pwm_ch` + `pwm_data_invert`) are read by exactly one of
the two `mt_irtx` families, the node/major/class all match, and the HAL is an
independent witness to the buffer layout. **High** that solution type 1 is
correct, from the driver returning 1 and never touching the carrier registers.

What remains genuinely unverified is everything downstream of "we wrote the
bytes the stock stack would write": that the PWM channel is `pwm_ch = 0`'s
physical pin, that the LED is wired and powered, and that the emitted carrier
duty and timing land within a real television's tolerance. None of that can be
known without the blaster and a target device. See the last section.

## The protocols

Pure encoders in `src/proto.rs`, each a function from address/command to a
carrier and a mark/space table, checked against the published specs (San
Bergmans' [sbprojects.net/knowledge/ir](https://www.sbprojects.net/knowledge/ir/)
for NEC/RC5/RC6/SIRC; the common LIRC `SAMSUNG32` timings for Samsung):

| protocol | carrier | codes |
|---|---|---|
| `nec` | 38 kHz | 8-bit address + 8-bit command, address auto-complemented |
| `nec-ext` | 38 kHz | 16-bit address + 8-bit command |
| `rc5` | 36 kHz | 5-bit address + 6-bit command, bi-phase, toggle bit |
| `rc6` | 36 kHz | mode 0: 8-bit address + 8-bit command, double-width toggle |
| `sony12/15/20` | 40 kHz | 7-bit command + 5/8-bit address (+8-bit extended) |
| `samsung` | 38 kHz | 8-bit address (sent twice) + 8-bit command |
| `raw` | caller's | a literal microsecond table |

Repeats are per-protocol: NEC sends its short 9 ms/2.25 ms/560 µs "ditto" frame
every 110 ms; the others resend the whole frame (SIRC every 45 ms, the RCs and
Samsung ~107–114 ms). `tx.rs` handles held keys with `--repeats N`.

One carrier note: the brief lists "NEC/Sony 38 kHz", but the SIRC specification's
carrier is **40 kHz**, and that is what a Sony set expects, so the Sony encoders
use 40 kHz. The `--dry-run` output always prints the carrier, so this is never
hidden; override it with `--carrier` if a particular set proves otherwise.

## Codesets

`couch-model`'s `Integration::Ir { codeset }` names *which* remote's codes a
device answers to. The codes themselves live in a small text table
(`src/codeset.rs`), one button per line:

```text
# lg-tv.codeset
power   nec   0x04 0x08
volup   nec   0x04 0x02
voldn   nec   0x04 0x03
```

Button names are free text, but the intended vocabulary is `couch-model`'s
`Action.command` (`on`/`off`/`volume:up`/…). The convention — documented here,
not baked into a shared type — is that `codeset` resolves to
`/opt/couch/ir/<codeset>.codeset`. **`couch-model` is unchanged**: it already
carries the one field this needs, and adding an IR-code table to a crate that
also builds for wasm would be the wrong place for it. `couch-ir` deliberately
does not depend on `couch-model`, the same separation `couch-voice` keeps.

## Building

```sh
cd clients
cargo build --release --target armv7-unknown-linux-musleabihf -p couch-ir
cargo test -p couch-ir          # host, no device needed
```

A static musl binary, `rust-lld`-linked, no cross sysroot — the same toolchain
constraints `docs/voice.md` describes. The release binary is ~342 KB. The only
dependency is `libc`, for `open`/`ioctl`/`write`/`close`.

## What is verified, and how

Proven by `cargo test -p couch-ir` (32 tests, host):

* **Every protocol encoder**, against exact spec tables — NEC's worked example
  (address 0x00, command 0x16), NEC extended, Samsung's symmetric leader and
  doubled address, Sony 12-bit, and the fully hand-derived RC5 and RC6 all-zero
  frames (checked against their canonical totals, 24003 µs and 52·444 µs).
* **The PWM buffer generation**, both solutions, against hand-computed buffers
  where the geometry is small enough to check bit by bit, plus word-boundary
  crossing and the little-endian serialisation.
* **The ioctl numbers**, against the vendor macros.
* **Repeat handling and solution selection**, by driving a recording fake
  through the real `transmit` path — NEC sends the ditto frame, resend protocols
  resend the whole frame, a single press never sleeps.

Not verifiable off-device, and needing the blaster:

* That `/dev/irtx` accepts the `write()` and the ioctls return what the source
  says (a wrong ioctl number would surface as `ENOTTY`, which `couch-ir` reports
  by name).
* That the emitted light actually switches a real television.
* That the carrier/duty are within a receiver's tolerance.

## Testing on the device

`couch-ir` prints the whole timing table with `--dry-run` before it will send
anything, precisely so this can be checked in stages.

**1. Confirm the encoding with no hardware** (any machine):

```sh
couch-ir send nec 0x04 0x08 --dry-run
```

Prints the carrier, the 67-entry NEC table, and the PWM buffer size. Diff that
table against a known-good NEC capture for the code, or against a logic-analyser
trace of the real send in step 3.

**2. Ask the driver what it is** (on the device). A real send reports the
solution the driver asked for; a quick way to see it is to send to a device that
is watching, or simply run a real send and read the last line:

```sh
couch-ir send nec 0x04 0x08          # prints: driver wanted PWM-only (type 1)
```

If this instead prints `ENOTTY` for `IRTX_IOC_GET_SOLUTTION_TYPE`, the kernel is
the older `mediatek,IRTX` driver, and `couch-ir` will have fallen back to type 0
— which is the correct choice for that driver, but flag it, because it means the
device is not the one this was researched against.

**3. Send at a real target.** Point the remote at a television whose codes you
know (the codeset above, or a raw NEC address/command from its service manual):

```sh
couch-ir send nec 0x04 0x08                 # LG power, for example
couch-ir codeset /opt/couch/ir/lg-tv.codeset power
couch-ir send nec 0x04 0x02 --repeats 5     # hold volume-up: 6 frames
```

Watch the set. If nothing happens, the fastest triage without a television is a
**bench IR receiver** (a TSOP38238 or a phone camera — most phone front cameras
see IR as a faint purple flicker) to confirm the LED is emitting at all, then a
**logic analyser** on the LED to compare the on-air marks against the `--dry-run`
table.

**4. If the marks are right but nothing responds**, the likely culprits, in
order: the carrier duty is off (try `--solution 0` to rule the baked carrier in
or out), the wrong `pwm_ch`, or the LED simply is not powered. Those are the
hardware unknowns this client cannot resolve alone, and the reason the write
path is one 200-line module: whatever the fix, it is one place.

## Open questions

* **Is the physical carrier really 38/40 kHz?** The math targets it and the HAL
  produces it, but only a scope on the LED confirms the duty and frequency the
  PWM actually emits.
* **Does `pwm_data_invert = 0` mean the LED idles low?** The driver inverts the
  buffer when the property is set; ours is 0, so we send it straight. If a
  capture shows inverted marks, the property or the wiring differs from the
  source and the one-line fix is in `pwm.rs`.
* **Held-key cadence.** `mt_irtx`'s own `write()` sleeps ~100 ms per call, so the
  real repeat period is at least that and `couch-ir`'s pacing can only add to it.
  Whether real devices care is untested.
