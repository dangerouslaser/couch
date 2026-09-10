//! Turning a [`Frame`] into the bytes `mt_irtx` wants on `write()`.
//!
//! Couch's waveform ABI is paired with its custom `couch_irtx` kernel driver.
//! Its carrier-scaled geometry derives from a later public MediaTek HAL, not
//! the original HA100 ROM. The stock HA100 HAL uses one-microsecond samples;
//! this implementation uses three samples per carrier period. It therefore
//! does not reproduce the stock firmware's buffers bit for bit. See
//! `docs/ha100-stock-ir-audit.md` for the measured and recovered evidence.
//!
//! # What the driver does with the buffer
//!
//! Little-endian `u32` waveform words play LSB first. Each sample lasts
//! `round(26 MHz / (3 * carrier_hz))` clocks: 228 clocks, about 8.77 us, at
//! 38 kHz. HDURATION and LDURATION each encode sample clocks minus one.
//! The final word is a duration trailer; Couch's driver excludes it from DMA.
//! This ABI is not compatible with an arbitrary stock MediaTek driver.
//!
//! # Two solutions, and which one this device uses
//!
//! The driver reports a "solution type" via `IRTX_IOC_GET_SOLUTTION_TYPE`:
//!
//! * [`Solution::IrtxPwm`] (0): the buffer is a plain on/off gate - all ones
//!   through a mark, all zeros through a space - and a hardware carrier
//!   generator modulates it. This is the fallback the HAL uses when the ioctl
//!   is unrecognised.
//! * [`Solution::PwmOnly`] (1): there is no carrier generator, so the carrier
//!   is *baked into the buffer* - during a mark, one tick on and two off per
//!   three-tick period (a ~33% duty 38 kHz-scaled square wave); during a space,
//!   nothing.
//!
//! Couch's HA100 driver returns 1 and never writes the
//! hardware IR registers - so [`Solution::PwmOnly`] is the real path, and
//! [`tx`](crate::tx) asks the driver at run time rather than assuming. Both are
//! implemented and tested because the query is one ioctl and getting the wrong
//! one is silently broken output, not an error.

use crate::proto::Frame;

/// Which waveform the driver wants, as reported by
/// [`crate::abi::IRTX_IOC_GET_SOLUTTION_TYPE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Solution {
    /// Type 0: on/off gate, hardware adds the carrier.
    IrtxPwm,
    /// Type 1: carrier baked into the buffer (this device).
    PwmOnly,
}

impl Solution {
    /// Map the ioctl's `unsigned int` to a solution, the way the HAL does:
    /// anything that is not 1 is treated as the type-0 fallback.
    pub fn from_raw(v: u32) -> Solution {
        if v == crate::abi::SOLUTION_PWM_ONLY {
            Solution::PwmOnly
        } else {
            Solution::IrtxPwm
        }
    }
}

/// The 26 MHz reference the PWM ticks off, in ticks per microsecond.
const CLOCK_MHZ: f64 = 26.0;
/// The HAL's carrier duty target: one third.
const DUTY: f64 = 0.33;

fn round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// Build the `write()` buffer for `frame` under `solution`, as `u32` words.
///
/// The last word is the trailer the driver expects (the total transmit time in
/// microseconds); every earlier word is waveform. This is exactly the layout
/// Couch's `ir_write` accepts, so [`to_bytes`] of the result is what goes on the
/// wire.
pub fn to_wave(frame: &Frame, solution: Solution) -> Vec<u32> {
    // Couch carrier-scaled geometry. h_l_period is reference clocks per
    // tick; a tick is frac microseconds; a carrier period is whole_cycle ticks.
    let carrier = frame.carrier_hz.max(1) as f64;
    let h_l_period = round(26_000_000.0 / (carrier * 3.0)).max(1);
    let frac = h_l_period as f64 / CLOCK_MHZ; // microseconds per tick
    let whole_cycle = round(26_000_000.0 / carrier / h_l_period as f64).max(1);
    let duty_cycle = round(whole_cycle as f64 * DUTY); // ticks on, per period

    // Round each duration to a whole number of carrier periods, in ticks. This
    // follows the public HAL's period-rounding approach with Couch sample units.
    let period_us = frac * whole_cycle as f64;
    let mut temp = Vec::with_capacity(frame.pattern_us.len());
    let mut total_tick: i64 = 0;
    let mut input_total_time: i64 = 0;
    for &d in &frame.pattern_us {
        input_total_time += d as i64;
        let ticks = round(d as f64 / period_us) * whole_cycle;
        temp.push(ticks);
        total_tick += ticks;
    }
    let total_time = round(total_tick as f64 * frac);

    // ceil(total_tick / 32) waveform words, plus one trailer word.
    let words = (total_tick as f64 / 32.0).ceil() as i64 + 1;
    let mut wave = vec![0u32; words as usize];

    let mut int_ptr = 0usize;
    let mut bit_ptr = 0i64;
    let mut level_on = true; // pattern_us starts on a mark
    for &ticks in &temp {
        match solution {
            Solution::IrtxPwm => {
                // Gate: every tick of a mark is a 1, every tick of a space a 0
                // (already zeroed).
                for _ in 0..ticks {
                    if level_on {
                        wave[int_ptr] |= 1u32 << bit_ptr;
                    }
                    bit_ptr += 1;
                    if bit_ptr == 32 {
                        bit_ptr = 0;
                        int_ptr += 1;
                    }
                }
            }
            Solution::PwmOnly => {
                // Carrier: within each whole_cycle-tick period of a mark, the
                // first duty_cycle ticks are on. Spaces stay zero. The cycle
                // counter advances at period boundaries, exactly as the HAL.
                let mut cycle_counter = 0i64;
                for j in 0..ticks {
                    if level_on
                        && cycle_counter * whole_cycle <= j
                        && j < cycle_counter * whole_cycle + duty_cycle
                    {
                        wave[int_ptr] |= 1u32 << bit_ptr;
                    }
                    bit_ptr += 1;
                    if j != 0 && (j + 1) % whole_cycle == 0 {
                        cycle_counter += 1;
                    }
                    if bit_ptr == 32 {
                        bit_ptr = 0;
                        int_ptr += 1;
                    }
                }
            }
        }
        level_on = !level_on;
    }

    // The trailer: the HAL stores max(total_time, input_total_time) here. The
    // Couch driver excludes this word before programming DMA, so it is never clocked
    // out as waveform - its presence is what makes that length come out right,
    // and its value is the transmit duration in microseconds. We reproduce both.
    let last = wave.len() - 1;
    wave[last] = total_time.max(input_total_time) as u32;
    wave
}

/// Serialise the wave words little-endian, which is the byte order the armv7
/// device's DMA reads and therefore what `write()` must receive.
pub fn to_bytes(words: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for w in words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(carrier: u32, pattern: &[u32]) -> Frame {
        Frame {
            carrier_hz: carrier,
            pattern_us: pattern.to_vec(),
        }
    }

    /// A two-entry pattern at 38 kHz where the geometry is easy to do by hand:
    /// h_l_period 228, frac 8.769 us, whole_cycle 3, duty_cycle 1. Each 26 us
    /// duration rounds to one carrier period = 3 ticks, so total_tick = 6,
    /// buffer = 2 words, trailer = round(6*8.769)=53.
    #[test]
    fn irtx_pwm_gate_is_all_ones_through_a_mark() {
        // Mark 26 us then space 26 us. Type 0: bits 0,1,2 set (mark), 3,4,5
        // clear (space) -> word0 = 0b111 = 7.
        let w = to_wave(&frame(38_000, &[26, 26]), Solution::IrtxPwm);
        assert_eq!(w, vec![7, 53]);
    }

    #[test]
    fn pwm_only_bakes_a_one_third_carrier() {
        // Same frame, type 1: within the mark's one 3-tick period, only the
        // first tick is on -> word0 = 0b001 = 1; space stays 0.
        let w = to_wave(&frame(38_000, &[26, 26]), Solution::PwmOnly);
        assert_eq!(w, vec![1, 53]);
    }

    #[test]
    fn carrier_geometry_holds_the_duty_across_a_longer_mark() {
        // Two carrier periods of mark (52 us) then one of space (26 us).
        // Type 1: ticks 0 and 3 on (one per period), rest off -> 0b001001 = 9.
        let w = to_wave(&frame(38_000, &[52, 26]), Solution::PwmOnly);
        assert_eq!(w[0], 0b001001);
        // total_tick = 6+3 = 9; ceil(9/32)+1 = 2 words.
        assert_eq!(w.len(), 2);
        // Type 0 gates the whole 52 us mark on: 6 ticks -> 0b111111 = 63.
        let g = to_wave(&frame(38_000, &[52, 26]), Solution::IrtxPwm);
        assert_eq!(g[0], 0b111111);
    }

    #[test]
    fn bit_stream_crosses_word_boundaries() {
        // A mark long enough to fill more than one 32-bit word, gated on. 32
        // ticks is 32 bits; make the mark >= 33 ticks so word 1 gets a bit.
        // 33 ticks must round to whole periods (11 periods = 33 ticks): pick a
        // duration near 11 * 26.3 us = 289 us.
        let w = to_wave(&frame(38_000, &[289]), Solution::IrtxPwm);
        assert_eq!(w[0], 0xFFFF_FFFF); // first 32 ticks all on
        assert_eq!(w[1] & 1, 1); // 33rd tick on
    }

    #[test]
    fn solution_from_raw_follows_the_hal_fallback() {
        assert_eq!(Solution::from_raw(1), Solution::PwmOnly);
        assert_eq!(Solution::from_raw(0), Solution::IrtxPwm);
        assert_eq!(Solution::from_raw(99), Solution::IrtxPwm);
    }

    #[test]
    fn to_bytes_is_little_endian() {
        assert_eq!(to_bytes(&[0x0403_0201]), vec![0x01, 0x02, 0x03, 0x04]);
        assert_eq!(to_bytes(&[7, 53]).len(), 8);
    }

    #[test]
    fn a_full_nec_frame_produces_a_sane_buffer() {
        // Not an exact-value test - that is proven above and in proto - but a
        // guard that a real frame does not panic and is plausibly sized. NEC is
        // ~67.5 ms; at ~8.77 us/tick that is ~7700 ticks -> ~241 words + trailer.
        let m = crate::proto::nec(0x04, 0x08).unwrap();
        let w = to_wave(&m.frame, Solution::PwmOnly);
        assert!(w.len() > 200 && w.len() < 300, "got {} words", w.len());
        // The trailer is the transmit duration in microseconds, near the frame
        // airtime (~67 ms) rounded to whole carrier periods.
        let trailer = *w.last().unwrap();
        assert!(trailer > 60_000 && trailer < 75_000, "trailer {trailer}");
    }
}
