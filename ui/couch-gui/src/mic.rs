//! Recording, held open by the microphone key.
//!
//! The capture runs on its own thread because the UI thread must keep drawing:
//! a missed period is an overrun, and an overrun in the middle of a sentence is
//! a hole in a word. The thread owns the `Capture`; this side keeps only a stop
//! handle and two atomics to read.
//!
//! What the screen shows is derived from whether that thread still holds the
//! device, never from a flag someone remembered to set. `live` goes up before
//! anything is opened and is cleared by the worker itself, as its last act,
//! after the stream is stopped, the device closed and the file written. There
//! is no code path that starts a capture without the key going down, and no
//! instant in which the device is open and the indicator dark.
//!
//! Stopping is asynchronous, and it has to be. The worker is normally inside
//! ALSA's poll for up to one period, and up to its full deadline if the stream
//! has gone quiet, so waiting for it would stall the UI on every release -
//! a dropped frame in the middle of the animation that says the microphone
//! just closed. `stop()` therefore trips the stop handle and returns; the
//! worker's tail, in order, is:
//!
//! 1. leave the read loop - the next `read` after the handle is tripped stops
//!    the stream and returns 0;
//! 2. zero the meter, because no more audio is coming;
//! 3. drop the `Capture`, which closes the device;
//! 4. write the WAV, 0600, into a file that did not exist a moment ago;
//! 5. clear `live`.
//!
//! So the indicator stays lit for that tail - a few tens of milliseconds -
//! and goes out only once the microphone really is closed and the recording
//! really is complete. Lit slightly too long is the safe direction; dark while
//! the device is open is the one that must not happen.
//!
//! A microphone press that arrives while a previous recording is still in its
//! tail is ignored, with a line in the log. The device is still open, so
//! opening it again would fail anyway, and the alternatives - waiting on the
//! UI thread, or queueing a recording to start later - are both worse than a
//! press that did nothing while the indicator was visibly still on.

use std::fs::File;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use couch_voice::alsa::{Pcm, StopHandle, Wanted};
use couch_voice::Source;
use couch_voice::{abi, level::Meter, wav};

/// Where a finished recording lands. One file, overwritten: this is a scratch
/// buffer for whatever consumes speech next, not an archive.
pub const OUT: &str = "/tmp/couch-voice.wav";

const DEVICE: &str = "hw:0,1";
const RATE: u32 = 16_000;

/// Long enough for anything anyone says to a remote, short enough that a key
/// wedged under a sofa cushion is a nuisance rather than a recording of your
/// evening.
const LIMIT: Duration = Duration::from_secs(30);

pub struct Mic {
    live: Arc<AtomicBool>,
    /// The meter, 0..1000, as an integer because it is read every frame and
    /// f32 has no atomic.
    meter: Arc<AtomicU32>,
    stop: Option<StopHandle>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Mic {
    pub fn new() -> Mic {
        Mic {
            live: Arc::new(AtomicBool::new(false)),
            meter: Arc::new(AtomicU32::new(0)),
            stop: None,
            worker: None,
        }
    }

    pub fn recording(&self) -> bool {
        self.live.load(Ordering::Relaxed)
    }

    /// 0.0 to 1.0, for a level meter.
    pub fn meter(&self) -> f32 {
        self.meter.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn start(&mut self) {
        self.reap();
        if self.recording() {
            // Either a capture is running - in which case the key is already
            // down and this is not a new press - or the last one is still
            // closing the device and writing its file. Both mean the device is
            // not ours to open.
            println!("couch-gui: microphone busy; press ignored");
            return;
        }
        // Up before the device is opened, not after: between `configure`
        // returning and the flag being set there would otherwise be a running
        // stream with a dark indicator, which is the one ordering this module
        // exists to rule out.
        self.live.store(true, Ordering::Relaxed);
        self.meter.store(0, Ordering::Relaxed);
        let wanted = Wanted {
            rate: RATE,
            limit: LIMIT,
            sample_format: abi::FORMAT_S16_LE,
            ..Wanted::default()
        };
        let capture = match Pcm::open(DEVICE).and_then(|p| p.configure(&wanted)) {
            Ok(c) => c,
            Err(e) => {
                // Not fatal, and not silent: a remote whose microphone key does
                // nothing should say why in the log rather than look broken.
                self.live.store(false, Ordering::Relaxed);
                eprintln!("couch-gui: microphone unavailable: {e}");
                return;
            }
        };
        self.stop = Some(capture.stop_handle());

        let (live, meter) = (self.live.clone(), self.meter.clone());
        self.worker = Some(std::thread::spawn(move || {
            let mut capture = capture;
            let mut buf = vec![0i16; capture.chunk_frames().max(1)];
            let mut samples = Vec::new();
            let mut m = Meter::default();
            while let Ok(n) = capture.read(&mut buf) {
                if n == 0 {
                    break;
                }
                m.feed(&buf[..n]);
                meter.store((m.level().meter() * 1000.0) as u32, Ordering::Relaxed);
                samples.extend_from_slice(&buf[..n]);
            }
            // The stream is over, whether the key came up, the 30s limit was
            // reached or the device failed. Nothing more will arrive, so the
            // meter says so.
            meter.store(0, Ordering::Relaxed);
            // Close the device before writing the file, not after: the
            // microphone should be off for the milliseconds spent on a WAV,
            // and `live` cannot come down until this has happened.
            drop(capture);
            write_out(OUT, &samples);
            // Last, so that "the indicator went out" means the device is
            // closed and the file is complete, in that order.
            live.store(false, Ordering::Relaxed);
        }));
    }

    /// Ends the recording without waiting for it. Returns immediately - the
    /// worker keeps `live` up until the device is closed and the file written,
    /// so the indicator, not this call, is what tracks the microphone.
    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.stop();
        }
    }

    /// Join a worker that has already finished. `live` is the last thing it
    /// clears, so a worker that has cleared it is at the end of its function
    /// and this cannot wait on ALSA or on a file.
    fn reap(&mut self) {
        if !self.recording() {
            if let Some(w) = self.worker.take() {
                let _ = w.join();
            }
        }
    }
}

/// The recording, into a file created for this user alone.
///
/// 0600, like the pairing PIN and the Home Assistant token, and unlinked first
/// so that the mode of a file somebody else left at this path cannot survive.
/// `create_new` rather than truncate for the same reason: after the unlink,
/// anything already at that name is not ours and is not to be written through.
fn write_out(path: &str, samples: &[i16]) {
    let _ = std::fs::remove_file(path);
    let file = File::options()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path);
    let writer = match file {
        Ok(f) => wav::Writer::from_file(f, path, RATE, 1),
        Err(e) => {
            eprintln!("couch-gui: cannot create {path}: {e}");
            return;
        }
    };
    let done = writer.and_then(|mut w| {
        w.write(samples)?;
        w.finish()
    });
    match done {
        Ok(()) => println!(
            "couch-gui: recorded {:.1}s to {path}",
            samples.len() as f32 / RATE as f32
        ),
        Err(e) => eprintln!("couch-gui: cannot write {path}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recording of a room is not something everyone with a shell gets to
    /// read, and the mode of whatever was sitting at that path before must not
    /// be what it ends up with.
    #[test]
    fn the_recording_is_written_0600_over_whatever_was_there() {
        use std::os::unix::fs::PermissionsExt;

        let path = "/tmp/couch-gui-mic-test.wav";
        let _ = std::fs::remove_file(path);
        std::fs::write(path, b"a stale recording anyone could read").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).unwrap();

        write_out(path, &[1i16, -1, 1]);

        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode {mode:o}");
        assert_eq!(wav::Reader::open(path).unwrap().samples, vec![1, -1, 1]);
        std::fs::remove_file(path).unwrap();
    }
}
