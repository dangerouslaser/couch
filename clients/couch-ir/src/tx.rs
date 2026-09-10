//! The one module that talks to the blaster.
//!
//! Everything above [`crate::proto`] and [`crate::pwm`] is arithmetic; this is
//! the seam where it becomes light. It exists as its own small file so the ABI
//! (the part we could not test without the device) is corrected in one place if
//! a hardware run proves it wrong, and so the encoders can never accidentally
//! grow a dependency on `/dev/irtx`.
//!
//! The raw syscalls sit behind the [`Blaster`] trait: [`Irtx`] is the real
//! device, and the tests drive a recording fake through the exact same
//! [`transmit`] path, so repeat handling and solution selection are proven on
//! the host with no blaster present.
//!
//! # Held keys
//!
//! [`transmit`] sends the frame once and then `repeats` more times. What it
//! sends for the repeats is the protocol's business ([`Repeat`]): NEC sends its
//! short ditto frame, everyone else resends the whole command. Between frames
//! it subtracts the previous frame's nominal airtime from the protocol's
//! start-to-start period. A driver's completion overhead can lengthen that
//! cadence; this pacing cannot shorten a blocking write. For single presses
//! (the common case, `repeats = 0`) none of this runs.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::Duration;

use crate::abi;
use crate::error::{Error, Result};
use crate::proto::{Message, Repeat};
use crate::pwm::{self, Solution};

/// The raw operations a blaster must supply. Three calls, matching the three
/// the vendor HAL makes: ask which waveform it wants, set the carrier, write a
/// wave. Opening is each implementation's own business.
pub trait Blaster {
    /// The node/name to put in an error, e.g. `/dev/irtx`.
    fn device(&self) -> &str;
    /// Which waveform encoding the driver wants. Implementations apply the HAL
    /// fallback (an unrecognised query means [`Solution::IrtxPwm`]).
    fn solution(&mut self) -> io::Result<Solution>;
    /// Set the subcarrier. Implementations tolerate a driver that ignores it.
    fn set_carrier(&mut self, hz: u32) -> io::Result<()>;
    /// Write one wave buffer (`u32` words) and block until it has gone out.
    fn write_wave(&mut self, words: &[u32]) -> io::Result<()>;
}

/// A summary of what [`transmit`] did, for the CLI to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    /// Total frames put out, including the first and every repeat.
    pub frames: u32,
    /// Words in the first frame's buffer (what a dry-run would also show).
    pub words: usize,
    /// The solution the driver asked for.
    pub solution: Solution,
}

/// The real blaster: an open `/dev/irtx`.
pub struct Irtx {
    fd: OwnedFd,
    path: String,
}

impl Irtx {
    /// Open the blaster. `O_RDWR` because the driver's `open` is what the HAL
    /// uses and the node is 0660 root:root; on this device everything runs as
    /// root so that is not a further privilege.
    pub fn open(path: &str) -> Result<Irtx> {
        let cpath = std::ffi::CString::new(path)
            .map_err(|_| Error::Encode(format!("device path {path:?} has a NUL")))?;
        // O_CLOEXEC so a blaster fd never leaks into a child; the driver is
        // single-open and a leaked fd would wedge it.
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(Error::Irtx {
                device: path.to_string(),
                call: "open",
                source: io::Error::last_os_error(),
            });
        }
        Ok(Irtx {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
            path: path.to_string(),
        })
    }

    fn raw(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl Blaster for Irtx {
    fn device(&self) -> &str {
        &self.path
    }

    fn solution(&mut self) -> io::Result<Solution> {
        let mut v: u32 = 0;
        match abi::ioctl_u32(self.raw(), abi::IRTX_IOC_GET_SOLUTTION_TYPE, &mut v) {
            Ok(()) => Ok(Solution::from_raw(v)),
            // An mt_irtx that does not implement the query is the old
            // register/"IRTX+PWM" driver; the HAL treats that as type 0.
            Err(e) if e.raw_os_error() == Some(libc::ENOTTY) => Ok(Solution::IrtxPwm),
            Err(e) => Err(e),
        }
    }

    fn set_carrier(&mut self, hz: u32) -> io::Result<()> {
        let mut v = hz;
        match abi::ioctl_u32(self.raw(), abi::IRTX_IOC_SET_CARRIER_FREQ, &mut v) {
            Ok(()) => Ok(()),
            // The PWM-only driver bakes the carrier into the buffer and does
            // not implement this ioctl. That is expected, not an error.
            Err(e) if e.raw_os_error() == Some(libc::ENOTTY) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn write_wave(&mut self, words: &[u32]) -> io::Result<()> {
        let bytes = pwm::to_bytes(words);
        let mut off = 0;
        while off < bytes.len() {
            let n = unsafe {
                libc::write(
                    self.raw(),
                    bytes[off..].as_ptr() as *const libc::c_void,
                    bytes.len() - off,
                )
            };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(e);
            }
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "irtx write returned 0",
                ));
            }
            off += n as usize;
        }
        Ok(())
    }
}

/// Send `message` once, then `repeats` more times, pacing between frames.
///
/// Uses a real sleep; see [`transmit_with`] to inject one (the tests do, so
/// they prove the repeat logic without waiting).
pub fn transmit<B: Blaster>(blaster: &mut B, message: &Message, repeats: u32) -> Result<Sent> {
    transmit_with(blaster, message, repeats, std::thread::sleep)
}

/// As [`transmit`], with the inter-frame wait injected.
pub fn transmit_with<B, S>(
    blaster: &mut B,
    message: &Message,
    repeats: u32,
    mut sleep: S,
) -> Result<Sent>
where
    B: Blaster,
    S: FnMut(Duration),
{
    // Own the device name up front so error-building never re-borrows the
    // blaster while a mutable call is in flight.
    let dev = blaster.device().to_string();
    let irtx_err = |call: &'static str, e: io::Error| Error::Irtx {
        device: dev.clone(),
        call,
        source: e,
    };

    let solution = blaster
        .solution()
        .map_err(|e| irtx_err("IRTX_IOC_GET_SOLUTTION_TYPE", e))?;
    blaster
        .set_carrier(message.frame.carrier_hz)
        .map_err(|e| irtx_err("IRTX_IOC_SET_CARRIER_FREQ", e))?;

    // First frame.
    let first_wave = pwm::to_wave(&message.frame, solution);
    blaster
        .write_wave(&first_wave)
        .map_err(|e| irtx_err("write", e))?;
    let mut frames = 1u32;

    // Repeats. The repeat frame and cadence come from the protocol.
    let (repeat_frame, period_ms) = match &message.repeat {
        Repeat::Resend { period_ms } => (&message.frame, *period_ms),
        Repeat::Ditto { frame, period_ms } => (frame, *period_ms),
    };
    if repeats > 0 {
        let repeat_wave = pwm::to_wave(repeat_frame, solution);
        // write_wave blocks for the frame that just finished. Subtract that
        // frame's airtime, not the upcoming repeat's: NEC's first frame is
        // much longer than its ditto. Preserve microseconds when pacing.
        let period = Duration::from_millis(u64::from(period_ms));
        let mut previous_airtime = Duration::from_micros(u64::from(message.frame.duration_us()));
        let repeat_airtime = Duration::from_micros(u64::from(repeat_frame.duration_us()));
        for _ in 0..repeats {
            sleep(period.saturating_sub(previous_airtime));
            blaster
                .write_wave(&repeat_wave)
                .map_err(|e| irtx_err("write", e))?;
            frames += 1;
            previous_airtime = repeat_airtime;
        }
    }

    Ok(Sent {
        frames,
        words: first_wave.len(),
        solution,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto;

    /// Records what it was asked to do, so the repeat logic can be checked with
    /// no device. `solution_raw` lets a test pretend to be either driver.
    struct Fake {
        solution_raw: u32,
        carriers: Vec<u32>,
        writes: Vec<Vec<u32>>,
        sleeps: Vec<Duration>,
    }

    impl Fake {
        fn new(solution_raw: u32) -> Fake {
            Fake {
                solution_raw,
                carriers: Vec::new(),
                writes: Vec::new(),
                sleeps: Vec::new(),
            }
        }
    }

    impl Blaster for Fake {
        fn device(&self) -> &str {
            "/dev/fake"
        }
        fn solution(&mut self) -> io::Result<Solution> {
            Ok(Solution::from_raw(self.solution_raw))
        }
        fn set_carrier(&mut self, hz: u32) -> io::Result<()> {
            self.carriers.push(hz);
            Ok(())
        }
        fn write_wave(&mut self, words: &[u32]) -> io::Result<()> {
            self.writes.push(words.to_vec());
            Ok(())
        }
    }

    fn run(msg: &Message, repeats: u32, solution_raw: u32) -> Fake {
        let mut f = Fake::new(solution_raw);
        let mut sleeps = Vec::new();
        let sent = transmit_with(&mut f, msg, repeats, |d| sleeps.push(d)).unwrap();
        assert_eq!(sent.frames, repeats + 1);
        f.sleeps = sleeps;
        f
    }

    #[test]
    fn single_press_sends_one_frame_and_never_sleeps() {
        let m = proto::nec(0x04, 0x08).unwrap();
        let f = run(&m, 0, 1);
        assert_eq!(f.writes.len(), 1);
        assert!(f.sleeps.is_empty());
        assert_eq!(f.carriers, vec![38_000]);
    }

    #[test]
    fn nec_repeats_send_the_short_ditto_frame() {
        let m = proto::nec(0x04, 0x08).unwrap();
        let f = run(&m, 3, 1);
        // 1 command + 3 ditto frames.
        assert_eq!(f.writes.len(), 4);
        // The ditto frames are all identical and shorter than the command.
        assert!(f.writes[1] == f.writes[2] && f.writes[2] == f.writes[3]);
        assert!(f.writes[1].len() < f.writes[0].len());
        assert_eq!(f.sleeps.len(), 3);
    }

    #[test]
    fn nec_first_repeat_accounts_for_the_full_frame_then_uses_ditto_airtime() {
        let m = proto::nec(0x04, 0x08).unwrap();
        let f = run(&m, 3, 1);
        // NEC full frame: 67,980 us; ditto: 11,810 us. Each following
        // frame starts 110,000 us after the previous one, before driver overhead.
        assert_eq!(
            f.sleeps,
            vec![
                Duration::from_micros(42_020),
                Duration::from_micros(98_190),
                Duration::from_micros(98_190),
            ]
        );
        assert_eq!(f.writes.len(), 4);
    }

    #[test]
    fn repeat_gaps_preserve_fractional_milliseconds_and_saturate() {
        let mut m = proto::Message {
            frame: proto::Frame {
                carrier_hz: 38_000,
                pattern_us: vec![1_250],
            },
            repeat: Repeat::Resend { period_ms: 10 },
        };
        assert_eq!(run(&m, 2, 1).sleeps, vec![Duration::from_micros(8_750); 2]);
        m.repeat = Repeat::Resend { period_ms: 1 };
        let f = run(&m, 2, 1);
        assert_eq!(f.sleeps, vec![Duration::ZERO; 2]);
        assert_eq!(f.writes.len(), 3);
    }

    #[test]
    fn resend_protocols_repeat_the_whole_frame() {
        let m = proto::sony(12, 0x01, 0x01, 0).unwrap();
        let f = run(&m, 2, 1);
        assert_eq!(f.writes.len(), 3);
        // Every frame is the same full command for a resend protocol.
        assert!(f.writes[0] == f.writes[1] && f.writes[1] == f.writes[2]);
    }

    #[test]
    fn the_two_solutions_produce_different_buffers() {
        let m = proto::nec(0x04, 0x08).unwrap();
        let gate = run(&m, 0, 0); // IrtxPwm
        let baked = run(&m, 0, 1); // PwmOnly
        assert_ne!(gate.writes[0], baked.writes[0]);
    }
}
