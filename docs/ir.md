# Infrared

Couch transmits per-device codes through the HA100's onboard blaster. It does
not provide IR learning. `clients/couch-ir/` contains the Rust protocol encoders,
waveform generator and `/dev/irtx` transport.

## Driver status

The first live bringup kernel (`3dee4cfb`) registers `/dev/irtx` and accepts
the solution/carrier ioctls. A single zero waveform returned ETIMEDOUT after
53,114 µs; GUI heartbeats advanced and the device did not hang. Optical
transmission remains unvalidated. The bringup configuration enables
`CONFIG_MTK_IRTX_PWM_SUPPORT` and uses Couch's `couch_irtx.c` on MT6580. Its
miscdevice requests `/dev/irtx`, dynamic minor, mode **0600** (runtime mdev
currently sets **0660**, observed major/minor 10:61); the stock major and
`mt_irtx` class must not be hardcoded. Probe waits for the PWM controller and
does not start a transmission.

The effective device tree confirms `mediatek,irtx-pwm`, `pwm_ch = 0` and
`pwm_data_invert = 0`; the existing overlay supplies these properties. The PWM
controller is bound at `11008000.PWM`. Preserve the current DTB and overlay.

A diagnostic follow-up (`146bbaeb`) observes the powered channel on failure:
PWM enable/clock selection, interrupt enable/status, control and duration
registers, buffer word count and requested/sent waveform counters. It reads
only the active channel and shared controller, before acknowledgement and
clock shutdown. It changes no interrupt enable or transmit behavior.
`sent_waves=1` with no finish status would support investigating masked status;
zero progress instead points toward clock/configuration/DMA. Neither is assumed
in advance. The legacy computed-clock helper is not used as frequency evidence.

An earlier driver revision hung during transmission. Compilation and successful
probe do **not** validate LED output, carrier frequency, completion interrupts,
or display coexistence. Physical validation remains pending; see below.

## Couch's waveform ABI

Couch explicitly pairs its Rust generator with its custom kernel driver.
Compatibility with an arbitrary stock MediaTek driver is not established.

| Operation | Value | Behavior |
|---|---|---|
| Set carrier | `0x40045200`, pointer to u32 | 10–100 kHz, stored per open descriptor |
| Get solution | `0x80045201`, pointer to u32 | returns 1, PWM-only |
| Write | little-endian u32 words | waveform followed by one duration word |

Each sample bit lasts `round(26 MHz / (3 × carrier))` reference clocks. At
38 kHz this is 228 clocks, approximately 8.77 µs. A mark uses one high sample
and two low samples, about 33% duty. `HDURATION` and `LDURATION` each contain
sample clocks minus one. Words play least-significant bit first.

The final u32 is the declared frame duration in microseconds. The driver
**subtracts four bytes before allocating/copying DMA data**, then programs
`BUF0_SIZE = waveform_words - 1`. The register itself counts words minus one;
it does not remove a trailer. Only waveform bytes are inverted when requested.

Writes are serialized; carrier settings on another open descriptor cannot
change the current frame. Both declared duration and computed waveform length
are bounded to about one second, with up to one padded word. Completion polls
have a computed timeout plus 50 ms scheduling slack. The channel is disabled
and drained before DMA memory is freed. The trailer never controls a timeout.

## Source evidence and correction

The [older MediaTek PWM driver](https://github.com/SoCXin/MT6737/blob/89eaf42ad72e6fcde9b1179c7be30e05968fe63c/linux/kernel/drivers/misc/mediatek/irtx/mt6735/mt_irtx_pwm.c)
uses one-microsecond samples, does not implement carrier selection and clocks
all supplied words. The [later consumer-IR HAL](https://github.com/StyxProject/vendor_mediatek_opensource/blob/856297f9007ca310ae33d8d54881789a15135926/hardware/consumerir/src/consumerir.c)
generates carrier-scaled samples and a duration trailer. These are useful
references, but **not a verified compatible pair**. Earlier documentation
incorrectly treated their buffer formats as interchangeable and assumed the
register's minus-one encoding removed the trailer. The Couch driver corrects
that mismatch; optical measurement is still required.

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

**2. Validate registration without sending.** After a coordinated candidate
boot, inspect `/dev/irtx`, the driver probe log and normal display/touch/wake
behavior. An ioctl-only query can confirm solution 1 without writing a frame.
Do not use a power command as a driver-presence check.

**3. Coordinate one measured transmission.** Keep recovery access available,
choose a verified command for the actual target, and capture the blaster with a
receiver or oscilloscope. Compare carrier, duty and mark/space timing with the
dry run. Check the kernel log and display afterward. Do not retry a timed-out
frame automatically; it may already have emitted light.

**4. Validate repeats and coexistence.** Only after a single frame succeeds,
check held-key cadence, simultaneous display activity, wake/sleep and separate
callers with different carrier frequencies. The Couch driver polls completion;
it does not retain the vendor driver's fixed 100 ms post-send sleep.

## Bounded bringup probe

Build the diagnostic independently of the GUI:

```sh
cd clients
cargo build -p couch-ir --release --target armv7-unknown-linux-musleabihf --example irtx_probe
cargo run -p couch-ir --example irtx_probe -- --pulse --dry-run
```

The ARM executable is `clients/target/armv7-unknown-linux-musleabihf/release/examples/irtx_probe`.
On the device, its default operation only opens `/dev/irtx`, queries solution 1
and sets 38 kHz on that descriptor. `--zero` and `--pulse` are explicit write
operations; `--dry-run` opens nothing for any mode.

- `--zero`: writes `[0x00000000, 281]`, one zero waveform word and a duration trailer.
- `--pulse`: writes `[0x49249249, 281]`, eleven high samples spaced three ticks
  apart, ending low. This is a short carrier burst, not a television command.
- Both send exactly eight bytes in **one syscall with no retry**, even on EINTR
  or a short write. At 228 clocks per sample the actual waveform lasts about
  281 µs. Logs flush before the write and report return value, errno and elapsed
  time afterward. A successful return proves driver completion, not optical output.

Run registration-only checks before zero-waveform, pulse and finally a verified
real target command. Collect kernel messages, elapsed time, advancing GUI
heartbeat and display/wake state between stages. Stop after an error.

Before the first physical test, retain a verified normal boot image and recovery
partition backup. The boot-health process eventually clears the recovery BCB;
wait for that process to finish before arming recovery for a risky write, and
verify its value. Restore normal boot intent after the controlled test. A shell
`timeout` cannot recover a CPU wedged in MMIO; watchdog/physical recovery must
remain available. USB enumeration alone does not prove a responsive kernel.

The historical failure was a four-byte zero waveform on `15b9bb34`, followed by
watchdog reboot. Corrected clock ordering did not eliminate it. No captured
trace proves its exact cause, so do not attribute it solely to the ABI mismatch.
The candidate's minimum valid frame is eight bytes including the trailer.

## Remaining hardware checks

- Confirm PWM0 reaches the IR LED and its idle polarity is correct.
- Measure 36/38/40 kHz carrier and approximately 33% duty.
- Confirm completion status and bounded timeout behavior without a bus hang.
- Verify display PWM, touch and wake remain stable during and after sending.
- Provision verified LG toggle/discrete codes before enabling IR power actions;
  a default preference is not a bundled, validated code database.

## Offline library and imports

The configuration server embeds a pinned, source-attributed library. Browse it
through the web UI by brand, device type and model, assign source commands to
remote functions, then save a named local codeset. Existing IR devices can be
reassigned without replacing their room or device identity. LG power setup
uses the same picker but only accepts `power`, `power-on` and `power-off`.
Importing, browsing and saving never transmit. An explicit installed-command
Test request sends one press with no retry; a successful driver write is not
proof that an appliance received the signal.

Authenticated endpoints are `GET /api/ir/catalog`, `GET /api/ir/catalog/:id`,
`POST /api/ir/import` (`format`, `name`, `text`), and
`GET/PUT /api/ir/codesets/:id` (`text` on PUT). `GET /api/ir/codesets` lists local
sets. `POST /api/ir/codesets/:id/test` takes an exact installed `command` name.
Catalog responses distinguish supported commands and unsupported reasons,
include source/license details, and report `physically_verified: false`.
`blaster_available` means `/dev/irtx` is registered; it does not mean optical
output or appliance compatibility has been tested.

Local files live beside configuration in `ir/<id>.codeset` (normally
`/opt/couch/ir`). IDs use lowercase letters, digits, hyphens and underscores,
at most 64 characters. Writes are atomic; traversal and symlink reads are
rejected. A codeset is at most 256 KiB with 256 unique command names.
Existing `<button> <protocol> <address> <command>` lines remain supported.
Raw captures use `<button> raw <carrier-hz> <comma-separated-microseconds>`:
20–60 kHz, 1–1024 positive alternating mark/space durations, at most 500 ms.
The transmitter uses 33% duty cycle; incompatible imported raw duty cycles
are rejected rather than changed silently.

Flipper `.ir` Version 1 imports use four-byte little-endian addresses/commands.
NEC, compatible NECext, RC5/RC6, SIRC12/15/20 and Samsung32 map to Couch
encoders; other parsed protocols remain visible but cannot be assigned.
Flipper NECext carries a 16-bit command: conversion requires its second byte
to be the first byte's inverse, because Couch generates that inverse itself.
See the [official Flipper format](https://github.com/flipperdevices/flipperzero-firmware/blob/dev/documentation/file_formats/InfraredFileFormats.md)
and [NEC decoder](https://github.com/flipperdevices/flipperzero-firmware/blob/dev/lib/infrared/encoder_decoder/nec/infrared_decoder_nec.c).
The initial 44-model catalog's licensing boundary and refresh procedure are
recorded in [catalog provenance](../daemon/couch-confd/assets/ir/README.md).
User imports are local data; importing does not grant redistribution rights.
