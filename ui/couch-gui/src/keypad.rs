//! The hardware keypad.
//!
//! Two evdev nodes matter on this device: mt_gpio_kpd and mtk-kpd. Both are
//! read, because which one a given key arrives on is not obvious and a press on
//! the wrong one simply vanishes.
//!
//! Autorepeat is disabled in the device tree, so holding a key repeats only
//! because this implements it. The debounce is set in the odmdtbo overlay
//! (lowered from 50ms to 8ms) - without that, a rapid double tap arrives as a
//! single press and the UI looks broken.

use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;

use slint::platform::Key;

pub const REPEAT_DELAY_MS: u64 = 400;
pub const REPEAT_RATE_MS: u64 = 70;

const NODES: [&str; 2] = ["/dev/input/event1", "/dev/input/event2"];

/// evdev's input_event is 16 bytes on 32-bit ARM: two 4-byte timeval words,
/// then type, code and value.
const EVENT_SIZE: usize = 16;

pub struct Keypad {
    files: Vec<File>,
    held: u16,
    held_since: u64,
    last_repeat: u64,
}

pub struct Press {
    pub key: Option<Key>,
    /// Microseconds between the kernel timestamping the event and us seeing it.
    pub latency_us: u64,
    pub repeat: bool,
}

impl Keypad {
    pub fn open() -> Self {
        let files = NODES
            .iter()
            .filter_map(|p| {
                let f = File::options().read(true).open(p).ok()?;
                unsafe {
                    let fd = f.as_raw_fd();
                    let fl = libc::fcntl(fd, libc::F_GETFL);
                    libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
                }
                Some(f)
            })
            .collect();
        Keypad { files, held: 0, held_since: 0, last_repeat: 0 }
    }

    pub fn device_count(&self) -> usize {
        self.files.len()
    }

    /// Non-blocking. Returns at most one press per call - the UI only needs the
    /// most recent, and draining a burst into a queue just adds latency.
    pub fn poll(&mut self) -> Option<Press> {
        let mut out: Option<Press> = None;
        let mut buf = [0u8; EVENT_SIZE * 32];

        for f in self.files.iter_mut() {
            let n = match f.read(&mut buf) {
                Ok(n) => n,
                Err(_) => continue,
            };
            for ev in buf[..n].chunks_exact(EVENT_SIZE) {
                let sec = u32::from_ne_bytes(ev[0..4].try_into().unwrap()) as u64;
                let usec = u32::from_ne_bytes(ev[4..8].try_into().unwrap()) as u64;
                let etype = u16::from_ne_bytes(ev[8..10].try_into().unwrap());
                let code = u16::from_ne_bytes(ev[10..12].try_into().unwrap());
                let value = i32::from_ne_bytes(ev[12..16].try_into().unwrap());
                if etype != 1 {
                    continue;
                }
                if value == 0 {
                    if code == self.held {
                        self.held = 0;
                    }
                    continue;
                }
                if value != 1 {
                    continue;
                }
                // evdev stamps against CLOCK_REALTIME, so latency has to be
                // measured on the same clock: subtracting an epoch from a
                // monotonic uptime saturates to zero and reads as "no latency".
                let stamp = sec * 1_000_000 + usec;
                self.held = code;
                self.held_since = now_monotonic_us();
                self.last_repeat = 0;
                out = Some(Press {
                    key: map_key(code),
                    latency_us: now_realtime_us().saturating_sub(stamp),
                    repeat: false,
                });
            }
        }

        if out.is_none() && self.held != 0 {
            let now = now_monotonic_us();
            let due = if self.last_repeat == 0 {
                self.held_since + REPEAT_DELAY_MS * 1000
            } else {
                self.last_repeat + REPEAT_RATE_MS * 1000
            };
            if now >= due {
                self.last_repeat = now;
                out = Some(Press { key: map_key(self.held), latency_us: 0, repeat: true });
            }
        }
        out
    }
}

fn map_key(code: u16) -> Option<Key> {
    match code {
        103 => Some(Key::UpArrow),      // KEY_UP
        108 => Some(Key::DownArrow),    // KEY_DOWN
        105 => Some(Key::LeftArrow),    // KEY_LEFT
        106 => Some(Key::RightArrow),   // KEY_RIGHT
        28 | 96 => Some(Key::Return),   // KEY_ENTER / KEY_KPENTER
        1 | 158 => Some(Key::Escape),   // KEY_ESC / KEY_BACK
        172 => Some(Key::Home),         // KEY_HOMEPAGE
        _ => None,
    }
}

pub fn now_monotonic_us() -> u64 {
    clock_us(libc::CLOCK_MONOTONIC)
}

pub fn now_realtime_us() -> u64 {
    clock_us(libc::CLOCK_REALTIME)
}

fn clock_us(which: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(which, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000
}
