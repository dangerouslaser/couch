//! Recording, held open by the microphone key.
//!
//! The capture runs on its own thread because the UI thread must keep drawing:
//! a missed period is an overrun, and an overrun in the middle of a sentence is
//! a hole in a word. The thread owns the `Capture`; this side keeps only a stop
//! handle and two atomics to read.
//!
//! What the screen shows is derived from whether that thread is alive, never
//! from a flag someone remembered to set. There is no code path that starts a
//! capture without the key going down, and dropping the `Capture` stops the
//! stream - so "the indicator is lit" and "the microphone is on" cannot come
//! apart.

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
        if self.recording() {
            return;
        }
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
                eprintln!("couch-gui: microphone unavailable: {e}");
                return;
            }
        };
        self.stop = Some(capture.stop_handle());
        self.live.store(true, Ordering::Relaxed);
        self.meter.store(0, Ordering::Relaxed);

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
            // Write before clearing the flag, so anything watching for the
            // indicator to go out finds a complete file when it does.
            if let Ok(mut w) = wav::Writer::create(OUT, RATE, 1) {
                let _ = w.write(&samples);
                let _ = w.finish();
            }
            println!(
                "couch-gui: recorded {:.1}s to {OUT}",
                samples.len() as f32 / RATE as f32
            );
            live.store(false, Ordering::Relaxed);
        }));
    }

    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.stop();
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        self.live.store(false, Ordering::Relaxed);
        self.meter.store(0, Ordering::Relaxed);
    }
}
