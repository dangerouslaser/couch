//! The ALSA kernel interface, as the kernel actually defines it.
//!
//! This is `include/uapi/sound/asound.h` transcribed, because the alternative
//! is linking alsa-lib and the alternative to that is a cross sysroot. See
//! `docs/voice.md` for the argument; the short version is that this device is
//! built as a static musl binary with `rust-lld` and no cross-gcc, and every
//! ALSA crate on crates.io binds libasound.
//!
//! Transcribing a kernel ABI by hand is exactly as dangerous as it sounds, so
//! none of it is guessed. `tools/alsa-abi-check.sh` compiles the assertions at
//! the bottom of this file as C `_Static_assert`s against the NDK's real
//! `sound/asound.h` for `armv7a-linux-androideabi` - the same ARM EABI the
//! device's musl uses - and the device's kernel is 3.18, whose `asound.h`
//! defines these five structs identically to the header that check uses.
//!
//! Two consequences of that shape are worth stating. `snd_pcm_uframes_t` is
//! `unsigned long`, so every struct here changes size on a 64-bit host and the
//! ioctl numbers change with it - which is why they are computed by `_ioc`
//! rather than written down. And `snd_pcm_status` is deliberately absent: it
//! grew a field in 4.x, so its ioctl number differs between kernels, and
//! nothing here needs it. Overruns are found by `readi` returning EPIPE.

use std::io;
use std::os::fd::RawFd;

use libc::{c_long, c_ulong};

/// `snd_pcm_uframes_t`. Frame counts and buffer sizes, `unsigned long` wide.
pub type Uframes = c_ulong;
/// `snd_pcm_sframes_t`. The same, signed, because a transfer returns -EPIPE.
pub type Sframes = c_long;

// --- ioctl encoding ---------------------------------------------------------

const DIR_NONE: u32 = 0;
const DIR_WRITE: u32 = 1;
const DIR_READ: u32 = 2;

/// `_IOC`. The direction is from userspace's point of view, so `_IOR` - the
/// caller reads - is `DIR_READ`.
const fn _ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | (nr as u32)
}

const fn _io(ty: u8, nr: u8) -> u32 {
    _ioc(DIR_NONE, ty, nr, 0)
}
const fn _ior(ty: u8, nr: u8, size: usize) -> u32 {
    _ioc(DIR_READ, ty, nr, size)
}
const fn _iowr(ty: u8, nr: u8, size: usize) -> u32 {
    _ioc(DIR_READ | DIR_WRITE, ty, nr, size)
}

const A: u8 = b'A'; // PCM devices
const U: u8 = b'U'; // control devices

pub const PCM_IOCTL_PVERSION: u32 = _ior(A, 0x00, 4);
pub const PCM_IOCTL_INFO: u32 = _ior(A, 0x01, size_of::<PcmInfo>());
pub const PCM_IOCTL_HW_REFINE: u32 = _iowr(A, 0x10, size_of::<HwParams>());
pub const PCM_IOCTL_HW_PARAMS: u32 = _iowr(A, 0x11, size_of::<HwParams>());
pub const PCM_IOCTL_HW_FREE: u32 = _io(A, 0x12);
pub const PCM_IOCTL_SW_PARAMS: u32 = _iowr(A, 0x13, size_of::<SwParams>());
pub const PCM_IOCTL_PREPARE: u32 = _io(A, 0x40);
pub const PCM_IOCTL_START: u32 = _io(A, 0x42);
pub const PCM_IOCTL_DROP: u32 = _io(A, 0x43);
pub const PCM_IOCTL_READI_FRAMES: u32 = _ior(A, 0x51, size_of::<Xferi>());

pub const CTL_IOCTL_PVERSION: u32 = _ior(U, 0x00, 4);
pub const CTL_IOCTL_CARD_INFO: u32 = _ior(U, 0x01, size_of::<CardInfo>());
pub const CTL_IOCTL_ELEM_LIST: u32 = _iowr(U, 0x10, size_of::<ElemList>());
pub const CTL_IOCTL_ELEM_INFO: u32 = _iowr(U, 0x11, size_of::<ElemInfo>());
pub const CTL_IOCTL_ELEM_READ: u32 = _iowr(U, 0x12, size_of::<ElemValue>());
pub const CTL_IOCTL_ELEM_WRITE: u32 = _iowr(U, 0x13, size_of::<ElemValue>());

// --- PCM --------------------------------------------------------------------

pub const STREAM_PLAYBACK: i32 = 0;
pub const STREAM_CAPTURE: i32 = 1;

/// `struct snd_pcm_info`. What a device calls itself, which is how a probe
/// tells `MultiMedia1_Capture` from `TDM_Debug_Record`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PcmInfo {
    pub device: u32,
    pub subdevice: u32,
    pub stream: i32,
    pub card: i32,
    pub id: [u8; 64],
    pub name: [u8; 80],
    pub subname: [u8; 32],
    pub dev_class: i32,
    pub dev_subclass: i32,
    pub subdevices_count: u32,
    pub subdevices_avail: u32,
    pub sync: [u8; 16],
    pub reserved: [u8; 64],
}

/// `struct snd_mask`, 256 bits of "which of these enum values will you take".
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Mask {
    pub bits: [u32; 8],
}

impl Mask {
    pub fn any() -> Mask {
        Mask { bits: [!0; 8] }
    }

    pub fn none() -> Mask {
        Mask::default()
    }

    pub fn only(value: u32) -> Mask {
        let mut m = Mask::none();
        m.set(value);
        m
    }

    pub fn set(&mut self, value: u32) {
        self.bits[(value >> 5) as usize] |= 1 << (value & 31);
    }

    pub fn has(&self, value: u32) -> bool {
        self.bits[(value >> 5) as usize] & (1 << (value & 31)) != 0
    }
}

pub const INTERVAL_OPENMIN: u32 = 1 << 0;
pub const INTERVAL_OPENMAX: u32 = 1 << 1;
pub const INTERVAL_INTEGER: u32 = 1 << 2;
pub const INTERVAL_EMPTY: u32 = 1 << 3;

/// `struct snd_interval`. The four flags are a bitfield in C; on little-endian
/// ARM they land in this order in the low bits of one word.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Interval {
    pub min: u32,
    pub max: u32,
    pub flags: u32,
}

impl Interval {
    pub fn any() -> Interval {
        Interval {
            min: 0,
            max: u32::MAX,
            flags: 0,
        }
    }

    pub fn exactly(v: u32) -> Interval {
        Interval {
            min: v,
            max: v,
            flags: INTERVAL_INTEGER,
        }
    }

    pub fn between(min: u32, max: u32) -> Interval {
        Interval {
            min,
            max,
            flags: INTERVAL_INTEGER,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.flags & INTERVAL_EMPTY != 0 || self.min > self.max
    }

    /// Whether refine narrowed this to a single value, which is what "the
    /// hardware settled on N" means.
    pub fn is_exact(&self) -> bool {
        self.min == self.max
    }
}

// Parameter indices, as the kernel numbers them. The masks come first and the
// intervals after, and the struct stores each family in its own array, so a
// parameter's index into that array is its number minus the family's first.
pub const P_ACCESS: usize = 0;
pub const P_FORMAT: usize = 1;
pub const P_SUBFORMAT: usize = 2;
pub const FIRST_MASK: usize = P_ACCESS;

pub const P_SAMPLE_BITS: usize = 8;
pub const P_FRAME_BITS: usize = 9;
pub const P_CHANNELS: usize = 10;
pub const P_RATE: usize = 11;
pub const P_PERIOD_TIME: usize = 12;
pub const P_PERIOD_SIZE: usize = 13;
pub const P_PERIOD_BYTES: usize = 14;
pub const P_PERIODS: usize = 15;
pub const P_BUFFER_TIME: usize = 16;
pub const P_BUFFER_SIZE: usize = 17;
pub const P_BUFFER_BYTES: usize = 18;
pub const P_TICK_TIME: usize = 19;
pub const FIRST_INTERVAL: usize = P_SAMPLE_BITS;

pub const ACCESS_MMAP_INTERLEAVED: u32 = 0;
pub const ACCESS_MMAP_NONINTERLEAVED: u32 = 1;
pub const ACCESS_MMAP_COMPLEX: u32 = 2;
pub const ACCESS_RW_INTERLEAVED: u32 = 3;
pub const ACCESS_RW_NONINTERLEAVED: u32 = 4;

pub const FORMAT_S8: u32 = 0;
pub const FORMAT_U8: u32 = 1;
pub const FORMAT_S16_LE: u32 = 2;
pub const FORMAT_S16_BE: u32 = 3;
pub const FORMAT_U16_LE: u32 = 4;
pub const FORMAT_U16_BE: u32 = 5;
pub const FORMAT_S24_LE: u32 = 6;
pub const FORMAT_S24_BE: u32 = 7;
pub const FORMAT_U24_LE: u32 = 8;
pub const FORMAT_U24_BE: u32 = 9;
pub const FORMAT_S32_LE: u32 = 10;
pub const FORMAT_S32_BE: u32 = 11;
pub const FORMAT_U32_LE: u32 = 12;
pub const FORMAT_U32_BE: u32 = 13;
pub const FORMAT_FLOAT_LE: u32 = 14;
pub const FORMAT_S24_3LE: u32 = 32;
pub const FORMAT_S24_3BE: u32 = 33;
pub const SUBFORMAT_STD: u32 = 0;

pub fn access_name(v: u32) -> &'static str {
    match v {
        ACCESS_MMAP_INTERLEAVED => "MMAP_INTERLEAVED",
        ACCESS_MMAP_NONINTERLEAVED => "MMAP_NONINTERLEAVED",
        ACCESS_MMAP_COMPLEX => "MMAP_COMPLEX",
        ACCESS_RW_INTERLEAVED => "RW_INTERLEAVED",
        ACCESS_RW_NONINTERLEAVED => "RW_NONINTERLEAVED",
        _ => "?",
    }
}

pub fn format_name(v: u32) -> &'static str {
    match v {
        FORMAT_S8 => "S8",
        FORMAT_U8 => "U8",
        FORMAT_S16_LE => "S16_LE",
        FORMAT_S16_BE => "S16_BE",
        FORMAT_U16_LE => "U16_LE",
        FORMAT_U16_BE => "U16_BE",
        FORMAT_S24_LE => "S24_LE",
        FORMAT_S24_BE => "S24_BE",
        FORMAT_U24_LE => "U24_LE",
        FORMAT_U24_BE => "U24_BE",
        FORMAT_S32_LE => "S32_LE",
        FORMAT_S32_BE => "S32_BE",
        FORMAT_U32_LE => "U32_LE",
        FORMAT_U32_BE => "U32_BE",
        FORMAT_FLOAT_LE => "FLOAT_LE",
        FORMAT_S24_3LE => "S24_3LE",
        FORMAT_S24_3BE => "S24_3BE",
        _ => "?",
    }
}

/// `struct snd_pcm_hw_params`. Three masks and twelve intervals describing
/// what both sides will accept, narrowed by the kernel until one configuration
/// is left.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HwParams {
    pub flags: u32,
    pub masks: [Mask; 3],
    pub mres: [Mask; 5],
    pub intervals: [Interval; 12],
    pub ires: [Interval; 9],
    /// Which parameters the caller is asking the kernel to refine.
    pub rmask: u32,
    pub cmask: u32,
    pub info: u32,
    pub msbits: u32,
    pub rate_num: u32,
    pub rate_den: u32,
    pub fifo_size: Uframes,
    pub reserved: [u8; 64],
}

impl HwParams {
    /// Everything permitted, which is what you hand to HW_REFINE to ask the
    /// hardware what it can do rather than to tell it what to do.
    pub fn any() -> HwParams {
        HwParams {
            flags: 0,
            masks: [Mask::any(); 3],
            mres: [Mask::none(); 5],
            intervals: [Interval::any(); 12],
            ires: [Interval::default(); 9],
            // Every parameter, so the kernel refines the lot.
            rmask: !0,
            cmask: 0,
            info: !0,
            msbits: 0,
            rate_num: 0,
            rate_den: 0,
            fifo_size: 0,
            reserved: [0; 64],
        }
    }

    pub fn mask(&self, param: usize) -> &Mask {
        &self.masks[param - FIRST_MASK]
    }

    pub fn set_mask(&mut self, param: usize, mask: Mask) {
        self.masks[param - FIRST_MASK] = mask;
    }

    pub fn interval(&self, param: usize) -> &Interval {
        &self.intervals[param - FIRST_INTERVAL]
    }

    pub fn set_interval(&mut self, param: usize, interval: Interval) {
        self.intervals[param - FIRST_INTERVAL] = interval;
    }
}

/// `struct snd_pcm_sw_params`. Only `avail_min` and the two thresholds matter
/// for capture; the rest is set the way tinyalsa sets it, which is the way
/// alsa-lib sets it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SwParams {
    pub tstamp_mode: i32,
    pub period_step: u32,
    pub sleep_min: u32,
    pub avail_min: Uframes,
    pub xfer_align: Uframes,
    pub start_threshold: Uframes,
    pub stop_threshold: Uframes,
    pub silence_threshold: Uframes,
    pub silence_size: Uframes,
    pub boundary: Uframes,
    pub proto: u32,
    pub tstamp_type: u32,
    pub reserved: [u8; 56],
}

impl Default for SwParams {
    fn default() -> SwParams {
        SwParams {
            tstamp_mode: 0,
            period_step: 1,
            sleep_min: 0,
            avail_min: 1,
            xfer_align: 1,
            start_threshold: 1,
            stop_threshold: 0,
            silence_threshold: 0,
            silence_size: 0,
            boundary: 0,
            proto: 0,
            tstamp_type: 0,
            reserved: [0; 56],
        }
    }
}

/// `struct snd_xferi`. `buf` is a userspace pointer the kernel copies into.
#[repr(C)]
pub struct Xferi {
    pub result: Sframes,
    pub buf: *mut libc::c_void,
    pub frames: Uframes,
}

// --- control ----------------------------------------------------------------

pub const ELEM_TYPE_BOOLEAN: i32 = 1;
pub const ELEM_TYPE_INTEGER: i32 = 2;
pub const ELEM_TYPE_ENUMERATED: i32 = 3;
pub const ELEM_TYPE_BYTES: i32 = 4;
pub const ELEM_TYPE_INTEGER64: i32 = 6;

pub const ELEM_IFACE_MIXER: i32 = 2;

pub const ELEM_ACCESS_READ: u32 = 1 << 0;
pub const ELEM_ACCESS_WRITE: u32 = 1 << 1;
pub const ELEM_ACCESS_INACTIVE: u32 = 1 << 8;

/// `struct snd_ctl_card_info`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CardInfo {
    pub card: i32,
    pub pad: i32,
    pub id: [u8; 16],
    pub driver: [u8; 16],
    pub name: [u8; 32],
    pub longname: [u8; 80],
    pub reserved: [u8; 16],
    pub mixername: [u8; 80],
    pub components: [u8; 128],
}

/// `struct snd_ctl_elem_id`. `numid` is the stable handle; the name is what a
/// human types.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ElemId {
    pub numid: u32,
    pub iface: i32,
    pub device: u32,
    pub subdevice: u32,
    pub name: [u8; 44],
    pub index: u32,
}

impl Default for ElemId {
    fn default() -> ElemId {
        ElemId {
            numid: 0,
            iface: ELEM_IFACE_MIXER,
            device: 0,
            subdevice: 0,
            name: [0; 44],
            index: 0,
        }
    }
}

/// `struct snd_ctl_elem_list`. Two round trips: once with `space` zero to
/// learn `count`, then once with a buffer that size.
#[repr(C)]
pub struct ElemList {
    pub offset: u32,
    pub space: u32,
    pub used: u32,
    pub count: u32,
    pub pids: *mut ElemId,
    pub reserved: [u8; 50],
}

/// The 128-byte union inside `snd_ctl_elem_info`, kept opaque and decoded by
/// hand. It has to carry the union's alignment - `long long` and `__u64` are
/// both in there - or every field after it moves.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct InfoValue(pub [u8; 128]);

/// `struct snd_ctl_elem_info`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ElemInfo {
    pub id: ElemId,
    pub ty: i32,
    pub access: u32,
    pub count: u32,
    pub owner: i32,
    pub value: InfoValue,
    pub dimen: [u16; 4],
    pub reserved: [u8; 56],
}

impl Default for ElemInfo {
    fn default() -> ElemInfo {
        ElemInfo {
            id: ElemId::default(),
            ty: 0,
            access: 0,
            count: 0,
            owner: 0,
            value: InfoValue([0; 128]),
            dimen: [0; 4],
            reserved: [0; 56],
        }
    }
}

impl ElemInfo {
    /// `value.integer.{min,max,step}`, three `long`s at the front of the union.
    pub fn integer_range(&self) -> (i64, i64, i64) {
        let w = size_of::<c_long>();
        let read = |i: usize| -> i64 {
            let mut b = [0u8; 8];
            b[..w].copy_from_slice(&self.value.0[i * w..(i + 1) * w]);
            match w {
                8 => i64::from_le_bytes(b),
                _ => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64,
            }
        };
        (read(0), read(1), read(2))
    }

    /// `value.enumerated.items`, the first `unsigned int` of the union.
    pub fn enum_items(&self) -> u32 {
        u32::from_le_bytes(self.value.0[0..4].try_into().unwrap())
    }

    /// `value.enumerated.item`, which is written before ELEM_INFO to ask for
    /// the name of one item. The kernel answers in `value.enumerated.name`.
    pub fn set_enum_item(&mut self, item: u32) {
        self.value.0[4..8].copy_from_slice(&item.to_le_bytes());
    }

    /// `value.enumerated.name`, 64 bytes after `items` and `item`.
    pub fn enum_name(&self) -> String {
        cstr(&self.value.0[8..72])
    }
}

/// The 512-byte union inside `snd_ctl_elem_value`, same reasoning as
/// `InfoValue`.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct ElemValueUnion(pub [u8; 512]);

/// `struct snd_ctl_elem_value`. The tail is a `struct timespec` plus reserved
/// bytes that together are always 128, whatever a `long` is here.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ElemValue {
    pub id: ElemId,
    pub indirect: u32,
    pub value: ElemValueUnion,
    pub tail: [u8; 128],
}

impl Default for ElemValue {
    fn default() -> ElemValue {
        ElemValue {
            id: ElemId::default(),
            indirect: 0,
            value: ElemValueUnion([0; 512]),
            tail: [0; 128],
        }
    }
}

impl ElemValue {
    /// `value.integer.value[i]` - and `value.enumerated.item[i]`, which is an
    /// `unsigned int` array rather than a `long` one, so it needs its own
    /// accessor on a 64-bit host.
    pub fn long_at(&self, i: usize) -> i64 {
        let w = size_of::<c_long>();
        let mut b = [0u8; 8];
        b[..w].copy_from_slice(&self.value.0[i * w..(i + 1) * w]);
        match w {
            8 => i64::from_le_bytes(b),
            _ => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64,
        }
    }

    pub fn set_long_at(&mut self, i: usize, v: i64) {
        let w = size_of::<c_long>();
        let b = v.to_le_bytes();
        self.value.0[i * w..(i + 1) * w].copy_from_slice(&b[..w]);
    }

    pub fn u32_at(&self, i: usize) -> u32 {
        u32::from_le_bytes(self.value.0[i * 4..i * 4 + 4].try_into().unwrap())
    }

    pub fn set_u32_at(&mut self, i: usize, v: u32) {
        self.value.0[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

// --- calling into the kernel ------------------------------------------------

/// One ioctl, with the errno turned into an `io::Error` and EINTR retried.
///
/// EINTR matters here: a capture read can sit in the kernel for a whole period
/// and any signal - a `SIGWINCH` from a terminal resize is enough - would
/// otherwise surface as a failed recording.
///
/// # Safety
///
/// `arg` must point at whatever the kernel expects for `request`, and stay
/// valid for the call.
pub unsafe fn ioctl(fd: RawFd, request: u32, arg: *mut libc::c_void) -> io::Result<i32> {
    loop {
        let r = libc::ioctl(fd, request as _, arg);
        if r >= 0 {
            return Ok(r);
        }
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINTR) {
            return Err(e);
        }
    }
}

/// Wait for the device to have something to say, or give up. Returns false on
/// timeout.
pub fn wait_readable(fd: RawFd, timeout_ms: i32) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if r > 0 {
            return Ok(true);
        }
        if r == 0 {
            return Ok(false);
        }
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINTR) {
            return Err(e);
        }
    }
}

/// A NUL-terminated ASCII field out of a kernel struct.
pub fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Copy a name into a fixed 44-byte field, truncating rather than overflowing.
/// ALSA control names are at most 44 bytes including the NUL, so anything
/// longer was never going to match a real control.
pub fn set_name(field: &mut [u8; 44], name: &str) {
    *field = [0; 44];
    let n = name.as_bytes();
    let n = &n[..n.len().min(43)];
    field[..n.len()].copy_from_slice(n);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout this file claims, checked against the kernel's own header by
    /// `tools/alsa-abi-check.sh` for 32-bit ARM. These assertions are the same
    /// numbers, so a careless edit here fails `cargo test` on any 32-bit
    /// target without needing the NDK.
    #[test]
    fn structs_match_the_kernel_abi() {
        assert_eq!(size_of::<PcmInfo>(), 288);
        assert_eq!(size_of::<Mask>(), 32);
        assert_eq!(size_of::<Interval>(), 12);
        assert_eq!(size_of::<CardInfo>(), 376);
        assert_eq!(size_of::<ElemId>(), 64);
        assert_eq!(size_of::<ElemInfo>(), 272);
        assert_eq!(size_of::<ElemValue>(), 712);

        // These two carry `unsigned long` members, so they are only this size
        // where a long is 32 bits - which is the device, and is what the C
        // check asserts.
        if size_of::<Uframes>() == 4 {
            assert_eq!(size_of::<HwParams>(), 604);
            assert_eq!(size_of::<SwParams>(), 104);
            assert_eq!(size_of::<Xferi>(), 12);
        }
    }

    /// The ioctl numbers for the device, as verified against the real header.
    /// A wrong number is the failure mode that looks like broken hardware:
    /// ENOTTY from a driver that is working perfectly.
    #[test]
    fn ioctl_numbers_are_the_kernels() {
        if size_of::<Uframes>() != 4 {
            return;
        }
        assert_eq!(PCM_IOCTL_PVERSION, 0x8004_4100);
        assert_eq!(PCM_IOCTL_INFO, 0x8120_4101);
        assert_eq!(PCM_IOCTL_HW_REFINE, 0xC25C_4110);
        assert_eq!(PCM_IOCTL_HW_PARAMS, 0xC25C_4111);
        assert_eq!(PCM_IOCTL_HW_FREE, 0x0000_4112);
        assert_eq!(PCM_IOCTL_SW_PARAMS, 0xC068_4113);
        assert_eq!(PCM_IOCTL_PREPARE, 0x0000_4140);
        assert_eq!(PCM_IOCTL_START, 0x0000_4142);
        assert_eq!(PCM_IOCTL_DROP, 0x0000_4143);
        assert_eq!(PCM_IOCTL_READI_FRAMES, 0x800C_4151);
        assert_eq!(CTL_IOCTL_PVERSION, 0x8004_5500);
        assert_eq!(CTL_IOCTL_CARD_INFO, 0x8178_5501);
        assert_eq!(CTL_IOCTL_ELEM_LIST, 0xC048_5510);
        assert_eq!(CTL_IOCTL_ELEM_INFO, 0xC110_5511);
        assert_eq!(CTL_IOCTL_ELEM_READ, 0xC2C8_5512);
        assert_eq!(CTL_IOCTL_ELEM_WRITE, 0xC2C8_5513);
    }

    #[test]
    fn masks_address_the_right_bit() {
        let mut m = Mask::none();
        m.set(FORMAT_S16_LE);
        assert_eq!(m.bits[0], 1 << 2);
        m.set(FORMAT_S24_3LE);
        assert_eq!(m.bits[1], 1);
        assert!(m.has(FORMAT_S16_LE) && m.has(FORMAT_S24_3LE));
        assert!(!m.has(FORMAT_S32_LE));
        assert!(Mask::any().has(FORMAT_FLOAT_LE));
    }

    #[test]
    fn names_are_truncated_not_overflowed() {
        let mut f = [0u8; 44];
        set_name(&mut f, "Audio_MicSource1_Setting");
        assert_eq!(cstr(&f), "Audio_MicSource1_Setting");
        set_name(&mut f, &"x".repeat(60));
        assert_eq!(cstr(&f).len(), 43);
    }
}
