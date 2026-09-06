//! Consumer-IR protocol encoders: a logical command to a mark/space table.
//!
//! Every function here is pure and hardware-free. The timings are the
//! published ones and the tests at the bottom assert whole tables against them,
//! so a careless edit to a constant fails `cargo test` on any host. Sources,
//! all consulted for this module:
//!
//! * NEC: San Bergmans, "NEC protocol", <https://www.sbprojects.net/knowledge/ir/nec.php>
//! * RC5: San Bergmans, "Philips RC-5", <https://www.sbprojects.net/knowledge/ir/rc5.php>
//! * RC6: San Bergmans, "Philips RC-6", <https://www.sbprojects.net/knowledge/ir/rc6.php>
//! * SIRC: San Bergmans, "Sony SIRC", <https://www.sbprojects.net/knowledge/ir/sirc.php>
//! * Samsung: the widely-used LIRC/`SAMSUNG32` timings, a NEC-shaped frame with
//!   a 4.5 ms leader mark and the address sent twice.
//!
//! A [`Frame`] is one transmission; a [`Message`] pairs the first frame with
//! how it repeats when a key is held (see [`Repeat`]).

use crate::error::{Error, Result};

/// A carrier frequency and its mark/space table. See the crate root for the
/// invariant (starts on a mark, alternates, microseconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub carrier_hz: u32,
    pub pattern_us: Vec<u32>,
}

/// How a frame repeats while its key is held down.
///
/// The two shapes are the two things real remotes do. NEC sends a short
/// distinct "I am still holding it" frame; almost everyone else just resends
/// the whole command. `period_ms` is the time from the start of one
/// transmission to the start of the next, which is the figure the protocol
/// specs quote and the one [`crate::tx`] paces against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    /// Resend the original frame every `period_ms`.
    Resend { period_ms: u32 },
    /// Send this shorter frame instead, every `period_ms` (NEC).
    Ditto { frame: Frame, period_ms: u32 },
}

/// A command ready to transmit: the frame, and its held-key behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub frame: Frame,
    pub repeat: Repeat,
}

/// The protocols this crate can encode. `Raw` carries a caller-supplied table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Nec,
    NecExt,
    Rc5,
    Rc6,
    Sony12,
    Sony15,
    Sony20,
    Samsung,
    Raw,
}

impl Protocol {
    /// The name used on the command line and in codesets.
    pub fn name(self) -> &'static str {
        match self {
            Protocol::Nec => "nec",
            Protocol::NecExt => "nec-ext",
            Protocol::Rc5 => "rc5",
            Protocol::Rc6 => "rc6",
            Protocol::Sony12 => "sony12",
            Protocol::Sony15 => "sony15",
            Protocol::Sony20 => "sony20",
            Protocol::Samsung => "samsung",
            Protocol::Raw => "raw",
        }
    }

    pub fn from_name(s: &str) -> Option<Protocol> {
        Some(match s {
            "nec" => Protocol::Nec,
            "nec-ext" | "nec-extended" => Protocol::NecExt,
            "rc5" => Protocol::Rc5,
            "rc6" => Protocol::Rc6,
            "sony12" | "sirc12" => Protocol::Sony12,
            "sony15" | "sirc15" => Protocol::Sony15,
            "sony20" | "sirc20" => Protocol::Sony20,
            "samsung" | "samsung32" => Protocol::Samsung,
            "raw" => Protocol::Raw,
            _ => return None,
        })
    }

    /// The carrier this protocol modulates at, in Hz. `Raw` has none of its own
    /// - the caller gives one - so this reports its most common value, 38 kHz.
    pub fn carrier_hz(self) -> u32 {
        match self {
            Protocol::Nec | Protocol::NecExt | Protocol::Samsung | Protocol::Raw => 38_000,
            Protocol::Rc5 | Protocol::Rc6 => 36_000,
            Protocol::Sony12 | Protocol::Sony15 | Protocol::Sony20 => 40_000,
        }
    }

    /// Every protocol, in a sensible order for `couch-ir list`.
    pub fn all() -> &'static [Protocol] {
        &[
            Protocol::Nec,
            Protocol::NecExt,
            Protocol::Rc5,
            Protocol::Rc6,
            Protocol::Sony12,
            Protocol::Sony15,
            Protocol::Sony20,
            Protocol::Samsung,
            Protocol::Raw,
        ]
    }

    /// One line for `couch-ir list`, naming what `address`/`command` mean.
    pub fn describe(self) -> &'static str {
        match self {
            Protocol::Nec => "NEC: 8-bit address + 8-bit command, address auto-complemented",
            Protocol::NecExt => "NEC extended: 16-bit address + 8-bit command",
            Protocol::Rc5 => "Philips RC-5: 5-bit address + 6-bit command, bi-phase",
            Protocol::Rc6 => "Philips RC-6 mode 0: 8-bit address + 8-bit command, bi-phase",
            Protocol::Sony12 => "Sony SIRC 12-bit: 5-bit address + 7-bit command",
            Protocol::Sony15 => "Sony SIRC 15-bit: 8-bit address + 7-bit command",
            Protocol::Sony20 => "Sony SIRC 20-bit: 5-bit address + 8-bit extended + 7-bit command",
            Protocol::Samsung => "Samsung 32-bit: 8-bit address (sent twice) + 8-bit command",
            Protocol::Raw => "raw: caller supplies the carrier and the mark/space table",
        }
    }
}

/// Encode `address`/`command` in `protocol` into a ready-to-send [`Message`].
///
/// `toggle` only matters for RC5/RC6, where it is the bit that flips between
/// distinct presses so a receiver can tell "pressed twice" from "held". The
/// caller owns it because only the caller knows whether this is a new press.
pub fn encode(protocol: Protocol, address: u32, command: u32, toggle: bool) -> Result<Message> {
    match protocol {
        Protocol::Nec => Ok(nec(address, command)?),
        Protocol::NecExt => Ok(nec_ext(address, command)?),
        Protocol::Rc5 => Ok(rc5(address, command, toggle)?),
        Protocol::Rc6 => Ok(rc6(address, command, toggle)?),
        Protocol::Sony12 => Ok(sony(12, address, command, 0)?),
        Protocol::Sony15 => Ok(sony(15, address, command, 0)?),
        Protocol::Sony20 => Ok(sony(20, address, command, 0)?),
        Protocol::Samsung => Ok(samsung(address, command)?),
        Protocol::Raw => Err(Error::Encode(
            "raw takes a timing table, not an address/command - use `couch-ir raw`".into(),
        )),
    }
}

fn fits(name: &str, value: u32, bits: u32) -> Result<()> {
    if value >> bits != 0 {
        return Err(Error::Encode(format!(
            "{name} {value:#x} does not fit in {bits} bits"
        )));
    }
    Ok(())
}

// --- NEC --------------------------------------------------------------------
//
// Carrier 38 kHz. A 9 ms mark and 4.5 ms space lead in; each bit is a 560 us
// mark and then a space that is 560 us for a 0 and 1690 us for a 1; a final
// 560 us mark closes the frame. Bytes go LSB first. Standard NEC sends the
// address, its bitwise complement, the command and its complement - the
// complements are the error check. A held key sends a 9 ms / 2.25 ms / 560 us
// "ditto" frame every 110 ms.

const NEC_LEAD_MARK: u32 = 9000;
const NEC_LEAD_SPACE: u32 = 4500;
const NEC_BIT_MARK: u32 = 560;
const NEC_ONE_SPACE: u32 = 1690;
const NEC_ZERO_SPACE: u32 = 560;
const NEC_REPEAT_SPACE: u32 = 2250;
const NEC_PERIOD_MS: u32 = 110;

/// Push one byte, LSB first, as NEC bit-mark + spaced pairs.
fn nec_push_byte(pattern: &mut Vec<u32>, byte: u8) {
    for i in 0..8 {
        pattern.push(NEC_BIT_MARK);
        pattern.push(if (byte >> i) & 1 == 1 {
            NEC_ONE_SPACE
        } else {
            NEC_ZERO_SPACE
        });
    }
}

fn nec_from_bytes(bytes: [u8; 4]) -> Frame {
    let mut pattern = vec![NEC_LEAD_MARK, NEC_LEAD_SPACE];
    for b in bytes {
        nec_push_byte(&mut pattern, b);
    }
    pattern.push(NEC_BIT_MARK); // stop bit
    Frame {
        carrier_hz: 38_000,
        pattern_us: pattern,
    }
}

fn nec_repeat_frame() -> Frame {
    Frame {
        carrier_hz: 38_000,
        pattern_us: vec![NEC_LEAD_MARK, NEC_REPEAT_SPACE, NEC_BIT_MARK],
    }
}

pub fn nec(address: u32, command: u32) -> Result<Message> {
    fits("address", address, 8)?;
    fits("command", command, 8)?;
    let a = address as u8;
    let c = command as u8;
    Ok(Message {
        frame: nec_from_bytes([a, !a, c, !c]),
        repeat: Repeat::Ditto {
            frame: nec_repeat_frame(),
            period_ms: NEC_PERIOD_MS,
        },
    })
}

pub fn nec_ext(address: u32, command: u32) -> Result<Message> {
    fits("address", address, 16)?;
    fits("command", command, 8)?;
    let lo = (address & 0xff) as u8;
    let hi = (address >> 8) as u8;
    let c = command as u8;
    Ok(Message {
        frame: nec_from_bytes([lo, hi, c, !c]),
        repeat: Repeat::Ditto {
            frame: nec_repeat_frame(),
            period_ms: NEC_PERIOD_MS,
        },
    })
}

// --- Samsung ----------------------------------------------------------------
//
// The same 560/1690 bit cell as NEC and the same 38 kHz, but the leader is a
// 4.5 ms mark and 4.5 ms space (equal, where NEC's mark is twice its space),
// and the four bytes are address, address again, command, ~command. The whole
// frame resends about every 108 ms when held.

const SAMSUNG_LEAD: u32 = 4500;
const SAMSUNG_PERIOD_MS: u32 = 108;

pub fn samsung(address: u32, command: u32) -> Result<Message> {
    fits("address", address, 8)?;
    fits("command", command, 8)?;
    let a = address as u8;
    let c = command as u8;
    let mut pattern = vec![SAMSUNG_LEAD, SAMSUNG_LEAD];
    for b in [a, a, c, !c] {
        nec_push_byte(&mut pattern, b);
    }
    pattern.push(NEC_BIT_MARK);
    Ok(Message {
        frame: Frame {
            carrier_hz: 38_000,
            pattern_us: pattern,
        },
        repeat: Repeat::Resend {
            period_ms: SAMSUNG_PERIOD_MS,
        },
    })
}

// --- bi-phase (Manchester) helpers for RC5/RC6 ------------------------------
//
// RC5 and RC6 encode each bit as a mid-bit transition rather than a duration,
// so a run of same-level half-bits merges into one longer mark or space. The
// two protocols disagree on which way a 1 goes and on the unit time, but the
// machinery - build a list of (level, half-duration) halves, then coalesce
// adjacent equal levels - is shared.

/// Append one bi-phase bit as two halves of `unit` each. `one_is_high_first`
/// picks the convention: RC5 sends a 1 as space-then-mark, RC6 as
/// mark-then-space.
fn biphase_bit(halves: &mut Vec<(bool, u32)>, bit: bool, unit: u32, one_is_high_first: bool) {
    let high_first = bit == one_is_high_first;
    if high_first {
        halves.push((true, unit));
        halves.push((false, unit));
    } else {
        halves.push((false, unit));
        halves.push((true, unit));
    }
}

/// Coalesce adjacent equal-level halves, drop a leading space (it is idle air
/// before the first mark), and return the durations. Asserts the result starts
/// on a mark and alternates, which is the [`Frame`] invariant.
fn coalesce(halves: &[(bool, u32)]) -> Vec<u32> {
    let mut runs: Vec<(bool, u32)> = Vec::new();
    for &(level, dur) in halves {
        match runs.last_mut() {
            Some(last) if last.0 == level => last.1 += dur,
            _ => runs.push((level, dur)),
        }
    }
    if matches!(runs.first(), Some((false, _))) {
        runs.remove(0);
    }
    let mut out = Vec::with_capacity(runs.len());
    for (i, (level, dur)) in runs.into_iter().enumerate() {
        debug_assert_eq!(level, i % 2 == 0, "coalesced pattern must alternate from a mark");
        out.push(dur);
    }
    out
}

// --- RC5 --------------------------------------------------------------------
//
// Carrier 36 kHz, half-bit 889 us (bit time 1778 us). Fourteen bits, MSB
// first: two start bits (both 1 for commands 0-63), a toggle bit, 5 address
// bits, 6 command bits. A 1 is space-then-mark. The frame resends about every
// 114 ms. (Extended RC5, which steals the second start bit for a 7th command
// bit, is not encoded here; commands are 0-63.)

const RC5_HALF: u32 = 889;
const RC5_PERIOD_MS: u32 = 114;

pub fn rc5(address: u32, command: u32, toggle: bool) -> Result<Message> {
    fits("address", address, 5)?;
    fits("command", command, 6)?;
    let mut bits: Vec<bool> = vec![true, true, toggle];
    for i in (0..5).rev() {
        bits.push((address >> i) & 1 == 1);
    }
    for i in (0..6).rev() {
        bits.push((command >> i) & 1 == 1);
    }
    let mut halves = Vec::new();
    for b in bits {
        biphase_bit(&mut halves, b, RC5_HALF, false); // 1 = low(space) first
    }
    Ok(Message {
        frame: Frame {
            carrier_hz: 36_000,
            pattern_us: coalesce(&halves),
        },
        repeat: Repeat::Resend {
            period_ms: RC5_PERIOD_MS,
        },
    })
}

// --- RC6 --------------------------------------------------------------------
//
// Carrier 36 kHz, unit t = 444 us. A leader of 6t mark and 2t space (not
// bi-phase), then bi-phase: a start bit (1), three mode bits (000 for mode 0),
// a toggle bit that is *double width* (2t per half), then 8 address and 8
// command bits, all MSB first. RC6 inverts RC5's convention: a 1 is
// mark-then-space. Resends about every 107 ms.

const RC6_T: u32 = 444;
const RC6_PERIOD_MS: u32 = 107;

pub fn rc6(address: u32, command: u32, toggle: bool) -> Result<Message> {
    fits("address", address, 8)?;
    fits("command", command, 8)?;
    // Leader is a plain mark/space, fed in as two halves so coalescing carries
    // it: 6t mark, 2t space.
    let mut halves: Vec<(bool, u32)> = vec![(true, 6 * RC6_T), (false, 2 * RC6_T)];
    biphase_bit(&mut halves, true, RC6_T, true); // start bit, 1 = high first
    for _ in 0..3 {
        biphase_bit(&mut halves, false, RC6_T, true); // mode 0
    }
    biphase_bit(&mut halves, toggle, 2 * RC6_T, true); // double-width toggle
    for i in (0..8).rev() {
        biphase_bit(&mut halves, (address >> i) & 1 == 1, RC6_T, true);
    }
    for i in (0..8).rev() {
        biphase_bit(&mut halves, (command >> i) & 1 == 1, RC6_T, true);
    }
    Ok(Message {
        frame: Frame {
            carrier_hz: 36_000,
            pattern_us: coalesce(&halves),
        },
        repeat: Repeat::Resend {
            period_ms: RC6_PERIOD_MS,
        },
    })
}

// --- Sony SIRC --------------------------------------------------------------
//
// Carrier 40 kHz (the SIRC spec's value; the project brief's "38 kHz" is a
// simplification - a 40 kHz remote's marks are what a Sony set expects, and the
// dry-run prints the carrier so it is never hidden). Unit 600 us. A 2.4 ms
// mark and 600 us space lead in; each bit is a mark - 1200 us for a 1, 600 us
// for a 0 - then a 600 us space. Bits go LSB first: command (7 bits) then
// address, whose width is what makes a frame 12, 15 or 20 bits. The whole frame
// repeats every 45 ms, and at least three times, which the tx layer handles.

const SIRC_UNIT: u32 = 600;
const SIRC_HEADER_MARK: u32 = 2400;
const SIRC_PERIOD_MS: u32 = 45;

fn sirc_push_bit(pattern: &mut Vec<u32>, bit: bool) {
    pattern.push(if bit { 2 * SIRC_UNIT } else { SIRC_UNIT });
    pattern.push(SIRC_UNIT);
}

/// `bits` is 12, 15 or 20; `extended` is only used by the 20-bit form.
pub fn sony(bits: u32, address: u32, command: u32, extended: u32) -> Result<Message> {
    let addr_bits = match bits {
        12 | 20 => 5,
        15 => 8,
        _ => return Err(Error::Encode(format!("Sony SIRC is 12, 15 or 20 bits, not {bits}"))),
    };
    fits("command", command, 7)?;
    fits("address", address, addr_bits)?;
    if bits == 20 {
        fits("extended", extended, 8)?;
    } else if extended != 0 {
        return Err(Error::Encode(
            "only 20-bit SIRC has an extended field".into(),
        ));
    }

    let mut pattern = vec![SIRC_HEADER_MARK, SIRC_UNIT];
    for i in 0..7 {
        sirc_push_bit(&mut pattern, (command >> i) & 1 == 1);
    }
    for i in 0..addr_bits {
        sirc_push_bit(&mut pattern, (address >> i) & 1 == 1);
    }
    if bits == 20 {
        for i in 0..8 {
            sirc_push_bit(&mut pattern, (extended >> i) & 1 == 1);
        }
    }
    Ok(Message {
        frame: Frame {
            carrier_hz: 40_000,
            pattern_us: pattern,
        },
        repeat: Repeat::Resend {
            period_ms: SIRC_PERIOD_MS,
        },
    })
}

// --- Raw --------------------------------------------------------------------

/// A caller-supplied table, validated only for the frame invariant.
pub fn raw(carrier_hz: u32, pattern_us: Vec<u32>) -> Result<Message> {
    if carrier_hz == 0 {
        return Err(Error::Encode("raw carrier must be non-zero Hz".into()));
    }
    if pattern_us.is_empty() {
        return Err(Error::Encode("raw needs at least one duration".into()));
    }
    if pattern_us.contains(&0) {
        return Err(Error::Encode("raw durations must be non-zero microseconds".into()));
    }
    Ok(Message {
        frame: Frame {
            carrier_hz,
            pattern_us,
        },
        // A raw table has no protocol repeat semantics; resending the whole
        // thing on a sensible cadence is the only defensible held-key answer.
        repeat: Repeat::Resend { period_ms: 110 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build the mark/space runs of a byte the long way, so the tests do not
    // reuse the code under test.
    fn nec_byte(byte: u8) -> Vec<u32> {
        let mut v = Vec::new();
        for i in 0..8 {
            v.push(560);
            v.push(if (byte >> i) & 1 == 1 { 1690 } else { 560 });
        }
        v
    }

    /// The brief's worked example: NEC address 0x00, command 0x16.
    #[test]
    fn nec_matches_the_spec_example() {
        let m = nec(0x00, 0x16).unwrap();
        assert_eq!(m.frame.carrier_hz, 38_000);
        let mut expected = vec![9000, 4500];
        for b in [0x00u8, 0xFF, 0x16, 0xE9] {
            expected.extend(nec_byte(b));
        }
        expected.push(560); // stop bit
        assert_eq!(m.frame.pattern_us, expected);
        // 2 leader + 64 bit halves + 1 stop.
        assert_eq!(m.frame.pattern_us.len(), 67);
        assert_eq!(
            m.repeat,
            Repeat::Ditto {
                frame: Frame {
                    carrier_hz: 38_000,
                    pattern_us: vec![9000, 2250, 560]
                },
                period_ms: 110
            }
        );
    }

    #[test]
    fn nec_complements_the_address_and_command() {
        // 0x04 / 0x08 -> bytes 0x04, 0xFB, 0x08, 0xF7.
        let m = nec(0x04, 0x08).unwrap();
        let mut expected = vec![9000, 4500];
        for b in [0x04u8, 0xFB, 0x08, 0xF7] {
            expected.extend(nec_byte(b));
        }
        expected.push(560);
        assert_eq!(m.frame.pattern_us, expected);
    }

    #[test]
    fn nec_ext_sends_a_full_16_bit_address() {
        // Address 0x20DF (low 0xDF, high 0x20), command 0x10.
        let m = nec_ext(0x20DF, 0x10).unwrap();
        let mut expected = vec![9000, 4500];
        for b in [0xDFu8, 0x20, 0x10, 0xEF] {
            expected.extend(nec_byte(b));
        }
        expected.push(560);
        assert_eq!(m.frame.pattern_us, expected);
    }

    #[test]
    fn nec_rejects_out_of_range() {
        assert!(nec(0x100, 0).is_err());
        assert!(nec(0, 0x100).is_err());
        assert!(nec_ext(0x1_0000, 0).is_err());
    }

    #[test]
    fn samsung_leader_is_symmetric_and_address_is_doubled() {
        let m = samsung(0x07, 0x02).unwrap();
        let mut expected = vec![4500, 4500];
        for b in [0x07u8, 0x07, 0x02, 0xFD] {
            expected.extend(nec_byte(b));
        }
        expected.push(560);
        assert_eq!(m.frame.pattern_us, expected);
        assert_eq!(m.frame.carrier_hz, 38_000);
    }

    /// Sony 12-bit, command 0x01, address 0x01: header, then command LSB-first
    /// (1,0,0,0,0,0,0), then 5 address bits LSB-first (1,0,0,0,0).
    #[test]
    fn sony12_exact_table() {
        let m = sony(12, 0x01, 0x01, 0).unwrap();
        assert_eq!(m.frame.carrier_hz, 40_000);
        let mut e = vec![2400, 600];
        // command bit0 = 1
        e.extend([1200, 600]);
        // command bits 1..6 = 0
        for _ in 0..6 {
            e.extend([600, 600]);
        }
        // address bit0 = 1
        e.extend([1200, 600]);
        // address bits 1..4 = 0
        for _ in 0..4 {
            e.extend([600, 600]);
        }
        assert_eq!(m.frame.pattern_us, e);
        // header pair + 12 bits * 2 entries.
        assert_eq!(m.frame.pattern_us.len(), 2 + 12 * 2);
    }

    #[test]
    fn sony_widths_and_bounds() {
        assert!(sony(15, 0xFF, 0x7F, 0).is_ok());
        assert!(sony(12, 0x20, 0, 0).is_err()); // address needs 6 bits
        assert!(sony(15, 0x100, 0, 0).is_err()); // address needs 9 bits
        assert!(sony(12, 0, 0x80, 0).is_err()); // command needs 8 bits
        assert!(sony(12, 0, 0, 1).is_err()); // extended only on 20-bit
        assert!(sony(20, 0x01, 0x01, 0xAB).is_ok());
        assert!(sony(13, 0, 0, 0).is_err());
    }

    /// RC-5 address 0, command 0, toggle 0. Fourteen bits `1 1 0 0 ...`; the
    /// only place two like halves meet is the second start bit's mark running
    /// into the toggle/first-address run, giving one 1778 us mark. Hand-derived
    /// and cross-checked: 14 bits * 1778 us = 24892 us of Manchester, less the
    /// 889 us leading space we drop = 24003 us.
    #[test]
    fn rc5_all_zero_exact_table() {
        let m = rc5(0, 0, false).unwrap();
        assert_eq!(m.frame.carrier_hz, 36_000);
        let mut e = vec![889, 889, 1778];
        e.extend(std::iter::repeat_n(889, 23));
        assert_eq!(m.frame.pattern_us, e);
        assert_eq!(m.frame.pattern_us.iter().sum::<u32>(), 24003);
    }

    #[test]
    fn rc5_starts_on_a_mark_and_alternates() {
        // A payload with transitions everywhere still must begin with a mark.
        let m = rc5(0x15, 0x2A, true).unwrap();
        // No run may be zero, and the first is a mark by construction of the
        // coalescer (it debug-asserts alternation).
        assert!(m.frame.pattern_us.iter().all(|&d| d == 889 || d == 1778));
        assert!(!m.frame.pattern_us.is_empty());
    }

    #[test]
    fn rc5_bounds() {
        assert!(rc5(0x20, 0, false).is_err()); // address 6 bits
        assert!(rc5(0, 0x40, false).is_err()); // command 7 bits
        assert!(rc5(0x1F, 0x3F, false).is_ok());
    }

    /// RC-6 mode 0, address 0, command 0, toggle 0. Leader 6t/2t, then the
    /// bi-phase body; the only merged run is the start bit's trailing space
    /// into the first mode bit's leading space (2t = 888 us). Total is the
    /// canonical 52t = 23088 us for mode 0. Hand-derived.
    #[test]
    fn rc6_all_zero_exact_table() {
        let m = rc6(0, 0, false).unwrap();
        assert_eq!(m.frame.carrier_hz, 36_000);
        let mut e = vec![2664, 888, 444, 888, 444, 444, 444, 444, 444, 888, 888];
        e.extend(std::iter::repeat_n(444, 32));
        assert_eq!(m.frame.pattern_us, e);
        assert_eq!(m.frame.pattern_us.iter().sum::<u32>(), 23088);
    }

    #[test]
    fn rc6_toggle_widens_the_toggle_bit() {
        // With toggle set, the double-width toggle bit is mark-first, so the
        // run structure around it changes but the frame still totals 52t.
        let m = rc6(0, 0, true).unwrap();
        assert_eq!(m.frame.pattern_us.iter().sum::<u32>(), 23088);
        assert_eq!(m.frame.pattern_us[0], 2664); // leader mark intact
    }

    #[test]
    fn rc6_bounds() {
        assert!(rc6(0x100, 0, false).is_err());
        assert!(rc6(0, 0x100, false).is_err());
        assert!(rc6(0xFF, 0xFF, false).is_ok());
    }

    #[test]
    fn raw_validates() {
        assert!(raw(38_000, vec![9000, 4500, 560]).is_ok());
        assert!(raw(0, vec![100]).is_err());
        assert!(raw(38_000, vec![]).is_err());
        assert!(raw(38_000, vec![100, 0, 100]).is_err());
    }

    #[test]
    fn protocol_names_round_trip() {
        for p in Protocol::all() {
            assert_eq!(Protocol::from_name(p.name()), Some(*p));
        }
        assert_eq!(Protocol::from_name("sirc12"), Some(Protocol::Sony12));
        assert_eq!(Protocol::from_name("nonsense"), None);
    }

    #[test]
    fn every_frame_starts_on_a_mark() {
        // The invariant the pwm layer relies on: index 0 is a mark. We cannot
        // assert the value, but we can assert the encoders never produce an
        // empty frame and that the alternation holds via the coalescer's
        // debug_assert (exercised by the RC tests above).
        let msgs = [
            nec(1, 2).unwrap(),
            nec_ext(0x1234, 2).unwrap(),
            samsung(1, 2).unwrap(),
            rc5(1, 2, false).unwrap(),
            rc6(1, 2, false).unwrap(),
            sony(12, 1, 2, 0).unwrap(),
            sony(20, 1, 2, 3).unwrap(),
        ];
        for m in msgs {
            assert!(!m.frame.pattern_us.is_empty());
        }
    }
}
