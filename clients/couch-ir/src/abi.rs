//! The `mt_irtx` kernel interface, as the vendor driver actually defines it.
//!
//! This is not in mainline Linux. It is transcribed from MediaTek's own tree,
//! cross-checked against MediaTek's own Android HAL, both cited in `docs/ir.md`.
//! The device the HA100 carries is the PWM variant: device-tree node
//! `mt_irtx_pwm`, compatible `mediatek,irtx-pwm`, properties `pwm_ch` and
//! `pwm_data_invert` - which is exactly and only what `mt_irtx_pwm.c` reads. It
//! exposes a character device `/dev/irtx` (major 243) whose `write()` takes a
//! PWM sample buffer and whose ioctls set the carrier and report which of two
//! waveform encodings the driver wants. See [`crate::pwm`] for the buffer.
//!
//! Three things about this ABI are worth stating up front because they are the
//! ones that would silently break:
//!
//! * The ioctl argument is always a single `unsigned int`, which is 4 bytes on
//!   both the armv7 device and a 64-bit host. So unlike ALSA - whose ioctl
//!   numbers move with `sizeof(long)` - these numbers are *identical* on host
//!   and device, and the test below asserts them unconditionally.
//! * The transmit itself is `write()`, not an ioctl. The ioctls only configure.
//! * `IRTX_IOC_SET_CARRIER_FREQ` is accepted but, on the PWM-only driver this
//!   device runs, ignored: that driver bakes the carrier into the sample buffer
//!   in userspace (again, see [`crate::pwm`]). We still send it, because it is
//!   what the stock HAL sends and because a different `mt_irtx` build does use
//!   it, and a driver that does not recognise it answers `ENOTTY`, which we
//!   tolerate.

use std::io;
use std::os::fd::RawFd;

// --- ioctl encoding ---------------------------------------------------------
//
// The vendor header writes these with the kernel's `_IOW`/`_IOR` macros:
//
//     #define IRTX_IOC_SET_CARRIER_FREQ   _IOW('R', 0, unsigned int)
//     #define IRTX_IOC_GET_SOLUTTION_TYPE _IOR('R', 1, unsigned int)  [sic]
//     #define IRTX_IOC_SET_DUTY_CYCLE     _IOW('R', 2, unsigned int)
//     #define IRTX_IOC_SET_IRTX_LED_EN    _IOW('R', 10, unsigned int)
//
// so they are computed the same way, and the magic letter 'R' (0x52) and the
// 4-byte size are spelled out rather than trusted to a comment.

const DIR_WRITE: u32 = 1;
const DIR_READ: u32 = 2;

/// `_IOC`. Direction is from userspace's point of view: `_IOR` - the caller
/// reads a value back - is [`DIR_READ`].
const fn _ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | (nr as u32)
}

const fn _iow(ty: u8, nr: u8, size: usize) -> u32 {
    _ioc(DIR_WRITE, ty, nr, size)
}
const fn _ior(ty: u8, nr: u8, size: usize) -> u32 {
    _ioc(DIR_READ, ty, nr, size)
}

/// The driver's ioctl magic letter.
const R: u8 = b'R';
/// Every ioctl here passes a single `unsigned int`.
const UINT: usize = 4;

/// Set the subcarrier in Hz. Honoured by the register/"IRTX+PWM" driver;
/// accepted-and-ignored by the PWM-only driver on this device.
pub const IRTX_IOC_SET_CARRIER_FREQ: u32 = _iow(R, 0, UINT);

/// Ask the driver which waveform encoding it wants back: see
/// [`crate::pwm::Solution`]. The vendor spelling of the name is preserved.
pub const IRTX_IOC_GET_SOLUTTION_TYPE: u32 = _ior(R, 1, UINT);

/// Set the carrier duty cycle, packed as `(high << 16) | low` giving `low/high`;
/// the HAL sends `(1 << 16) | 3` for one-third. Ignored by this device's
/// driver, which hard-codes a third; sent only to match the stock HAL.
pub const IRTX_IOC_SET_DUTY_CYCLE: u32 = _iow(R, 2, UINT);

/// The transmit magic value returned by [`IRTX_IOC_GET_SOLUTTION_TYPE`] on the
/// PWM-only driver: 1. See [`crate::pwm::Solution`].
pub const SOLUTION_PWM_ONLY: u32 = 1;
/// The value the HAL falls back to when the ioctl is unrecognised: 0.
pub const SOLUTION_IRTX_PWM: u32 = 0;

/// The device node the driver creates, class `mt_irtx`, major 243.
pub const DEVICE_PATH: &str = "/dev/irtx";

// --- calling into the kernel ------------------------------------------------

/// One ioctl passing a `u32` by pointer, errno turned into an `io::Error` and
/// EINTR retried. Every `mt_irtx` ioctl has this shape.
///
/// # Safety
///
/// `arg` is copied from/to by the kernel as a `u32`; the reference must stay
/// valid for the call, which it trivially does.
pub fn ioctl_u32(fd: RawFd, request: u32, arg: &mut u32) -> io::Result<()> {
    loop {
        // `request as _`: musl types the second argument of ioctl as `c_int`
        // and glibc as `c_ulong`; the cast lets one line compile for both, the
        // same trick couch-voice's abi uses.
        let r = unsafe { libc::ioctl(fd, request as _, arg as *mut u32) };
        if r >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINTR) {
            return Err(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ioctl numbers, computed here and asserted against the values the
    /// vendor's `_IOW`/`_IOR` macros produce. A wrong number is the failure
    /// that looks like broken hardware: ENOTTY from a driver that is fine.
    ///
    /// These are size-independent (the arg is always a 4-byte `unsigned int`),
    /// so unlike the ALSA ioctls they are the same on the host and the device
    /// and this test needs no target guard.
    #[test]
    fn ioctl_numbers_are_the_vendors() {
        // dir<<30 | size<<16 | 'R'<<8 | nr, with 'R' = 0x52, size = 4.
        assert_eq!(IRTX_IOC_SET_CARRIER_FREQ, 0x4004_5200);
        assert_eq!(IRTX_IOC_GET_SOLUTTION_TYPE, 0x8004_5201);
        assert_eq!(IRTX_IOC_SET_DUTY_CYCLE, 0x4004_5202);
        // nr 10, kept here so a future edit of `_iow` is checked against it.
        assert_eq!(_iow(R, 10, UINT), 0x4004_520A);
    }
}
