//! The touchscreen.
//!
//! A TLSC6x capacitive controller behind mtk-tpd, speaking multitouch protocol
//! B. Measured on the device: it reports ABS_MT_POSITION_X/Y in panel pixels
//! already - X across 0..479, Y across 0..799 - so there is no calibration, no
//! scaling and no axis to invert. A tap at (293, 475) is that pixel.
//!
//! Only the first finger is followed. Everything this device shows is a list or
//! a grid of single targets, so a second contact has nothing to mean, and
//! tracking it would be state to keep correct for no behaviour.
//!
//! Protocol B reports only what changed, so the current position has to be
//! remembered between packets: a drag that moves in Y alone sends no X, and a
//! release sends neither. Events are therefore accumulated and only acted on at
//! EV_SYN, which is the point at which the kernel says the packet is complete.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Read;

/// Same 16-byte input_event as the keypad: two 4-byte timeval words, then type,
/// code and value.
const EVENT_SIZE: usize = 16;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;

const BTN_TOUCH: u16 = 0x14a;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;

pub struct Touch {
    file: Option<File>,
    // Keep every event from a read, including a release after its press.
    buffered: VecDeque<[u8; EVENT_SIZE]>,
    x: f32,
    y: f32,
    down: bool,
    /// Which slot the packet being accumulated refers to. Only slot 0 is acted
    /// on; a second finger's coordinates must not be mistaken for the first's.
    slot: i32,
    has_slots: bool,
    pending_move: bool,
    pending_edge: Option<bool>,
}

/// What one complete packet meant, in panel pixels.
pub enum Event {
    Pressed { x: f32, y: f32 },
    Moved { x: f32, y: f32 },
    Released { x: f32, y: f32 },
}

impl Touch {
    pub fn open() -> Self {
        let file = crate::evdev::open_named(&["mtk-tpd", "couch-tlsc6x"])
            .into_iter()
            .next();
        Touch {
            file,
            buffered: VecDeque::new(),
            x: 0.0,
            y: 0.0,
            down: false,
            slot: 0,
            has_slots: false,
            pending_move: false,
            pending_edge: None,
        }
    }

    pub fn present(&self) -> bool {
        self.file.is_some()
    }

    /// One completed packet, or None. Called once per frame like the keypad;
    /// the node is non-blocking, so a screen nobody is touching costs a read
    /// that returns EAGAIN.
    pub fn poll(&mut self) -> Option<Event> {
        let mut buf = [0u8; EVENT_SIZE * 32];
        loop {
            while let Some(raw) = self.buffered.pop_front() {
                if let Some(event) = self.apply(&raw) {
                    return Some(event);
                }
            }
            // The read is scoped so the file is not still borrowed when a
            // completed packet needs &mut self to settle.
            let n = {
                let file = self.file.as_mut()?;
                match file.read(&mut buf) {
                    Ok(n) if n >= EVENT_SIZE => n,
                    _ => return None,
                }
            };
            self.buffered.extend(
                buf[..n]
                    .chunks_exact(EVENT_SIZE)
                    .map(|raw| -> [u8; EVENT_SIZE] {
                        raw.try_into().expect("complete input event")
                    }),
            );
        }
    }

    /// One raw event. Returns something only at the EV_SYN that completes a
    /// packet worth reporting.
    fn apply(&mut self, chunk: &[u8]) -> Option<Event> {
        let kind = u16::from_le_bytes([chunk[8], chunk[9]]);
        let code = u16::from_le_bytes([chunk[10], chunk[11]]);
        let value = i32::from_le_bytes([chunk[12], chunk[13], chunk[14], chunk[15]]);

        match (kind, code) {
            (EV_ABS, ABS_MT_SLOT) => {
                self.has_slots = true;
                self.slot = value;
            }
            (EV_ABS, ABS_MT_POSITION_X) if self.slot == 0 => {
                self.x = value as f32;
                self.pending_move = true;
            }
            (EV_ABS, ABS_MT_POSITION_Y) if self.slot == 0 => {
                self.y = value as f32;
                self.pending_move = true;
            }
            // A tracking id of -1 ends the contact. BTN_TOUCH says the same
            // thing for the first finger, and both are sent, so the edge is
            // recorded once and whichever arrives first wins.
            (EV_ABS, ABS_MT_TRACKING_ID) if self.slot == 0 => {
                self.has_slots = true;
                self.pending_edge = Some(value >= 0);
            }
            // BTN_TOUCH describes *any* finger. With slots, it must not
            // resurrect slot 0 while only the second finger remains down.
            (EV_KEY, BTN_TOUCH) if !self.has_slots => self.pending_edge = Some(value != 0),
            (EV_SYN, 0) => return self.settle(),
            _ => {}
        }
        None
    }

    /// A packet has ended. At most one event comes out of it: a press or a
    /// release is more informative than the move that arrived with it, and the
    /// coordinates are carried on all three.
    fn settle(&mut self) -> Option<Event> {
        let moved = std::mem::take(&mut self.pending_move);
        let (x, y) = (self.x, self.y);
        match std::mem::take(&mut self.pending_edge) {
            Some(true) if !self.down => {
                self.down = true;
                Some(Event::Pressed { x, y })
            }
            Some(false) if self.down => {
                self.down = false;
                Some(Event::Released { x, y })
            }
            _ if moved && self.down => Some(Event::Moved { x, y }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
        let mut b = [0u8; EVENT_SIZE];
        b[8..10].copy_from_slice(&kind.to_le_bytes());
        b[10..12].copy_from_slice(&code.to_le_bytes());
        b[12..16].copy_from_slice(&value.to_le_bytes());
        b
    }

    /// Drives the real decoder, one raw event at a time, with no device.
    fn feed(t: &mut Touch, events: &[[u8; EVENT_SIZE]]) -> Vec<(String, f32, f32)> {
        events
            .iter()
            .filter_map(|e| t.apply(e))
            .map(|e| match e {
                Event::Pressed { x, y } => ("press".to_string(), x, y),
                Event::Moved { x, y } => ("move".to_string(), x, y),
                Event::Released { x, y } => ("release".to_string(), x, y),
            })
            .collect()
    }

    fn blank() -> Touch {
        Touch {
            file: None,
            buffered: VecDeque::new(),
            x: 0.0,
            y: 0.0,
            down: false,
            slot: 0,
            has_slots: false,
            pending_move: false,
            pending_edge: None,
        }
    }

    #[test]
    fn poll_keeps_the_release_in_the_same_read_as_a_press() {
        use std::io::Write;
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        use std::os::unix::net::UnixStream;

        let (reader, mut writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut t = blank();
        t.file = Some(unsafe { File::from_raw_fd(reader.into_raw_fd()) });
        writer
            .write_all(
                &[
                    ev(EV_ABS, ABS_MT_TRACKING_ID, 1),
                    ev(EV_ABS, ABS_MT_POSITION_X, 20),
                    ev(EV_ABS, ABS_MT_POSITION_Y, 30),
                    ev(EV_SYN, 0, 0),
                    ev(EV_ABS, ABS_MT_TRACKING_ID, -1),
                    ev(EV_SYN, 0, 0),
                ]
                .concat(),
            )
            .unwrap();
        assert!(matches!(
            t.poll(),
            Some(Event::Pressed { x: 20.0, y: 30.0 })
        ));
        assert!(matches!(
            t.poll(),
            Some(Event::Released { x: 20.0, y: 30.0 })
        ));
        assert!(t.poll().is_none());
        assert!(!t.down);
    }

    #[test]
    fn second_finger_does_not_resurrect_a_released_primary_contact() {
        let mut t = blank();
        feed(
            &mut t,
            &[ev(EV_ABS, ABS_MT_TRACKING_ID, 1), ev(EV_SYN, 0, 0)],
        );
        let out = feed(
            &mut t,
            &[
                ev(EV_ABS, ABS_MT_TRACKING_ID, -1),
                ev(EV_ABS, ABS_MT_SLOT, 1),
                ev(EV_ABS, ABS_MT_TRACKING_ID, 2),
                ev(EV_KEY, BTN_TOUCH, 1),
                ev(EV_SYN, 0, 0),
            ],
        );
        assert_eq!(out, vec![("release".to_string(), 0.0, 0.0)]);
        assert!(!t.down);
    }

    /// The exact shape the device produced, captured from /dev/input/event3.
    #[test]
    fn a_real_tap_becomes_one_press_and_one_release() {
        let mut t = blank();
        let out = feed(
            &mut t,
            &[
                ev(EV_ABS, ABS_MT_TRACKING_ID, 52),
                ev(EV_ABS, ABS_MT_POSITION_X, 293),
                ev(EV_ABS, ABS_MT_POSITION_Y, 475),
                ev(EV_KEY, BTN_TOUCH, 1),
                ev(EV_SYN, 0, 0),
                ev(EV_ABS, ABS_MT_TRACKING_ID, -1),
                ev(EV_KEY, BTN_TOUCH, 0),
                ev(EV_SYN, 0, 0),
            ],
        );
        assert_eq!(
            out,
            vec![
                ("press".to_string(), 293.0, 475.0),
                ("release".to_string(), 293.0, 475.0),
            ]
        );
    }

    /// Protocol B sends only what changed, so a drag that moves in one axis
    /// must keep the other.
    #[test]
    fn a_partial_packet_keeps_the_axis_it_did_not_mention() {
        let mut t = blank();
        feed(
            &mut t,
            &[
                ev(EV_ABS, ABS_MT_TRACKING_ID, 7),
                ev(EV_ABS, ABS_MT_POSITION_X, 100),
                ev(EV_ABS, ABS_MT_POSITION_Y, 200),
                ev(EV_KEY, BTN_TOUCH, 1),
                ev(EV_SYN, 0, 0),
            ],
        );
        let out = feed(
            &mut t,
            &[ev(EV_ABS, ABS_MT_POSITION_Y, 260), ev(EV_SYN, 0, 0)],
        );
        assert_eq!(out, vec![("move".to_string(), 100.0, 260.0)]);
    }

    #[test]
    fn a_second_finger_does_not_move_the_first() {
        let mut t = blank();
        feed(
            &mut t,
            &[
                ev(EV_ABS, ABS_MT_POSITION_X, 40),
                ev(EV_ABS, ABS_MT_POSITION_Y, 60),
                ev(EV_KEY, BTN_TOUCH, 1),
                ev(EV_SYN, 0, 0),
            ],
        );
        let out = feed(
            &mut t,
            &[
                ev(EV_ABS, ABS_MT_SLOT, 1),
                ev(EV_ABS, ABS_MT_POSITION_X, 400),
                ev(EV_ABS, ABS_MT_POSITION_Y, 700),
                ev(EV_SYN, 0, 0),
            ],
        );
        assert!(out.is_empty(), "slot 1 should say nothing about slot 0");
        assert_eq!((t.x, t.y), (40.0, 60.0));
    }

    /// BTN_TOUCH and the tracking id both say the contact ended; between them
    /// they must produce one release, not two.
    #[test]
    fn a_release_is_reported_once() {
        let mut t = blank();
        feed(
            &mut t,
            &[
                ev(EV_ABS, ABS_MT_POSITION_X, 10),
                ev(EV_ABS, ABS_MT_POSITION_Y, 20),
                ev(EV_KEY, BTN_TOUCH, 1),
                ev(EV_SYN, 0, 0),
            ],
        );
        let out = feed(
            &mut t,
            &[
                ev(EV_ABS, ABS_MT_TRACKING_ID, -1),
                ev(EV_KEY, BTN_TOUCH, 0),
                ev(EV_SYN, 0, 0),
                ev(EV_SYN, 0, 0),
            ],
        );
        assert_eq!(out, vec![("release".to_string(), 10.0, 20.0)]);
    }
}
