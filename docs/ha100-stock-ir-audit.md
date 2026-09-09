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
