# HA100 stock IR driver audit

The original HA100 kernel was inspected offline after Couch completed PWM
transfers but an LG B4 did not respond. No device commands, GPIO changes or
transmissions were performed during this audit.

## Inputs and method

The privately backed-up `kernel.bin` has SHA256
`25d39655548070233670b2c3218284c5c0ccfede46285438cb7b43a76e814998`.
Its gzip payload expands to 18,972,672 bytes. Symbol recovery using
[vmlinux-to-elf](https://github.com/marin-m/vmlinux-to-elf), version 1.2.2.post2,
found 121,470 symbols with base address `0xc0008000`. LLVM objdump resolved the
IR call paths. Extracted binaries, reconstructed ELF and disassembly remain
outside Git.

The original vendor `consumerir.mt6580.so` hash is
`05658d1ca0c03a13f16f74eb554170cb597cb7b42d187b7981c10da6e1ced8c8`.
Its device paths are `/dev/irtx` and `/dev/ir-learning`; no GPIO/sysfs enable path
was evident. Android's IR service launches the standard HAL; board init changes
only `/dev/irtx` ownership and permissions.

## Findings

| Stock function | Address | Observed behavior |
|---|---|---|
| `dev_char_ioctl` | `0xc04f9cc8` | Solution query returns 1; LED-enable copies its argument but performs no action; carrier setting is unimplemented |
| `dev_char_write` | `0xc04f9e8c` | DMA copy/inversion, IRQ enable 0/1, 26 MHz selection, IRQ acknowledgement, PWM setup, delays, DMA free and PWM disable |
| `irtx_probe` | `0xc04fa264` | Reads channel/inversion and registers the character device; no GPIO, pinctrl or regulator setup |
| `mt_pwm_power_on_hal` | `0xc04d6e40` | Enables controller/channel clocks; no LED-power operation |

Stock `irtx_pwm_config` at `0xc1160c50` selects channel 0, memory mode,
no divider, block clock, idle/guard 0, stop bit 31, high/low duration 25 and one
waveform. The duration registers therefore target one-microsecond samples.
Couch deliberately pairs carrier-scaled Rust samples with corresponding duration
registers instead; stock and Couch buffers are not interchangeable.

## Conclusion and limits

No missing HA100-specific LED-enable sequence was found in these paths. GPIO8's
inherited PWM_A mux and the measured 26 MHz zero-sample rate already agree with
Couch's expected routing/configuration. IRQ enable differs intentionally because
Couch polls completion without an unhandled interrupt; there is no evidence that
IRQ masking suppresses PWM output.

This does not prove LED wiring, supply, optical polarity or emitted carrier.
A receiver/scope or a camera verified to detect a known IR emitter is the next
useful measurement. Do not guess GPIO/regulator changes from another board's
source. See [IR validation](ir.md) for actual transfer results and the failed
LG reception test.

## Android application and HAL follow-up

The following inspection used the original images privately stored on Ollie,
without mounting or accessing the remote. Read-only `debugfs` extraction, JADX
and Capstone Thumb disassembly were used; APKs, native libraries and reconstructed
source remain outside Git. The inspected images have SHA256:

- `system.img`: `39ed365ce914c6fccb2469ca03cfcb037b752bdcb87fcae1f102c882aa350ff8`
- `vendor.img`: `de47882c6c32d2b009c65b83217aec2ff2960e21a2b1ff03744f8f437da587a9`

### Factory application paths

`priv-app/TestApp/TestApp.apk` (SHA256
`66430fab48544f827138a11456113eb7a096be4202f9711d24e1b2b58897079d`)
contains `com.aiks360.rct.ui.manual.InfraredActivity` and
`InfraredActivity2`. Both obtain `consumer_ir`, check emitter availability and
call `ConsumerIrManager.transmit(38000, pattern)` directly. No additional
emitter-enable operation appears in these recovered methods.

The second activity sends one frame on a fresh dpad key-down, ignoring Android
key repeats. Its six 71-duration fixtures contain a complete NEC frame followed
by approximately 39.7–39.8 ms silence and one NEC ditto. They decode to address
`0x01`, inverse `0xfe`, with commands `46/16/47/15/55/40` hex for
up/down/left/right/OK/back. These are not the LG address `0x04` candidates.
The first activity's differently labelled send/on/off controls use the same
array, so their labels do not establish discrete power commands. A separate
menu-key fixture contains much shorter, cycle-count-looking durations passed
straight to the microsecond API; it is not a trustworthy timing reference.

`launcher_ha100`'s `GPIOUtils` controls `/sys/class/leds/red/brightness` and
`button-backlight/brightness` from charging/backlight code. Its packaged native
libraries are Bugly components. Targeted searches of the launcher, FactoryMode,
EngineerMode and the extracted factory/custom/HID JNI libraries found no
alternate transmitter enable path. The HID library references `/dev/uhid`,
not an IR peripheral. Some JADX classes failed decompilation, so this is a
bounded negative finding, not proof that every OEM path was recovered.

The backed-up `SanytronRemote.apk` also uses `ConsumerIrManager`, but contains
Kodi/WebOS/room configuration code and Kotlin 2.2 metadata. It appears to be a
later application prototype; do not treat it as independent factory evidence.

### Actual vendor HAL timing

The HAL's transmit implementation begins at ELF virtual address `0xac8` and is
**Thumb code**. Disassembling this region as ARM gives misleading instructions.
The recovered solution-1 path establishes:

| Setting or operation | Evidence | Behavior |
|---|---|---|
| `irtx.hal.mode` | `0xc1c–0xc46` | Defaults to 0; positive values retain input durations |
| `irtx.hal.duty` | `0xc4e–0xc96` | Defaults to 25, but is only logged in this solution-1 path |
| Carrier period | `0xca0–0xcbc` | Round `1,000,000 / carrier_hz` to integer microseconds |
| Mode-0 duration conversion | `0xd16–0xd50` | Round every mark/space to a whole number of those periods |
| Actual carrier high time | `0xece–0xee8` | Round floating period times hardcoded `0.33`; literal at `0x10e0` |
| Carrier synthesis | `0xf28–0xf9e` | Start carrier phase again for each mark; pack sample bits into words |

At a requested 38 kHz, this produces 26-microsecond periods and 9-microsecond
high intervals: approximately 38.46 kHz and 34.6% duty, before driver inversion.
No overrides of these properties were found in the inspected system/vendor
property files or `etc/init/*.rc` files. The vendor IR service stanza starts
the standard HAL as system/system without a separate enable action. Boot image
properties were outside this follow-up's scope.

### Actionable comparison and remaining limits

Retain Couch's matched sample-buffer/kernel timing configuration. Its current
three-samples-per-carrier geometry differs from this stock implementation;
comments describing it as the stock HAL's exact geometry should be corrected.
Changing an Android duty property would not change this solution-1 waveform,
and Couch does not consume those Android properties.

When a verified optical receiver or scope is available, compare carrier,
polarity and mark/space timing first. A controlled stock-timing fixture must
pair one-microsecond samples with duration registers 25; never feed that buffer
to Couch's carrier-scaled register configuration. Separately compare an LG
candidate frame alone with the same frame followed by an NEC ditto. Repeating
full commands at one-second intervals does not reproduce the factory fixture.
Neither difference currently establishes the cause of failed LG reception.
