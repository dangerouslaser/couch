//! Infrared transmit for Couch: turn a logical button into light out of the
//! blaster on the top of the remote.
//!
//! Three layers, and only the last one touches hardware:
//!
//! * [`proto`] encodes a command in a consumer-IR protocol (NEC, RC5, RC6,
//!   Sony SIRC, Samsung) into a [`Frame`] - a carrier frequency and a table of
//!   mark/space durations in microseconds. These are pure functions checked
//!   against the published protocol timings in the module's own tests; they
//!   know nothing about MediaTek and could drive any blaster.
//! * [`pwm`] is the MediaTek-specific half: it turns a [`Frame`] into the exact
//!   PWM sample buffer the `mt_irtx` driver expects on `write()`. This is a
//!   line-for-line port of the vendor Android HAL's `signals_generate`, so the
//!   bytes we hand the kernel are the bytes the stock firmware handed it. It is
//!   also pure and tested.
//! * [`tx`] is the seam. It opens `/dev/irtx`, runs the two ioctls and the
//!   `write()`, and handles repeats. It is the *only* module that calls into
//!   the kernel, behind the [`tx::Blaster`] trait, so the parts that can be
//!   proven without the device (everything above) are, and the part that
//!   cannot is one small file that `docs/ir.md` says exactly how to test.
//!
//! [`abi`] is the `mt_irtx` ioctl interface transcribed from the vendor kernel,
//! the same way `couch-voice`'s `abi` transcribes ALSA. [`codeset`] is the glue
//! to `couch-model`'s `Integration::Ir { codeset }`: a small text table mapping
//! a button name to a protocol, address and command.

pub mod abi;
pub mod codeset;
pub mod error;
pub mod proto;
pub mod pwm;
pub mod tx;

pub use error::{Error, Result};
pub use proto::{Frame, Message, Protocol, Repeat};

/// A carrier frequency and the mark/space table that rides on it.
///
/// `pattern_us` alternates on/off durations in microseconds, **starting with a
/// mark** (carrier on) and running mark, space, mark, space... The last entry
/// may be either: NEC ends on a mark (its stop bit), Sony ends on a space (the
/// gap after the final bit). Nothing here is device-specific - the same table
/// would drive a lirc transmitter or an LED on a GPIO. `carrier_hz` is the
/// subcarrier the marks are modulated at: 38 kHz for NEC, 36 kHz for the RCs,
/// 40 kHz for Sony. See [`pwm`] for how that modulation is actually produced on
/// this hardware.
///
/// It lives in [`proto`]; this re-export is the type the whole crate passes
/// around.
impl Frame {
    /// Total airtime of one frame, marks plus spaces, in microseconds. Used to
    /// pace repeats and to print a sanity figure in `--dry-run`.
    pub fn duration_us(&self) -> u32 {
        self.pattern_us.iter().sum()
    }
}
