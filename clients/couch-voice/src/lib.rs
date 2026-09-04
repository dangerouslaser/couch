//! Voice capture and a Home Assistant Assist pipeline client for Couch.
//!
//! Hold a button, speak, and get back what was heard and what the house said
//! in reply. Two halves, joined by one trait so that each can be used and
//! tested without the other:
//!
//! * [`alsa`] records 16-bit mono PCM straight off an ALSA character device,
//!   with a level meter and a hard cap on length. [`wav`] writes what it hears
//!   to a file, so the microphone can be proven with no network involved.
//! * [`ha`] authenticates to Home Assistant, runs an Assist pipeline over its
//!   WebSocket API and surfaces the events - the transcription, the
//!   assistant's answer, any error. It reads from a [`Source`], and a WAV file
//!   is one, so the pipeline can be proven with no microphone involved.
//!
//! Recording only ever happens because something called
//! [`Pcm::configure`](alsa::Pcm::configure). There is no path in this crate
//! that opens a capture device on a timer, on a wake word, or on anything but
//! an explicit call, and every stream carries a limit it cannot exceed. See
//! `docs/voice.md` for what a user interface owes someone standing in front of
//! a device that is listening.

pub mod abi;
pub mod alsa;
pub mod ctl;
pub mod error;
pub mod ha;
pub mod level;
pub mod wav;
pub mod ws;

pub use error::{Error, Result};

/// Somewhere 16-bit mono audio comes from: a live capture, or a file.
///
/// This is the seam. `Capture` implements it and so does `wav::Reader`, which
/// is what lets the Home Assistant client be exercised end to end on a laptop
/// with a recording, and the capture path be exercised on the device with
/// nothing to talk to.
pub trait Source {
    /// Samples per second. Home Assistant is told this, and resamples if it
    /// is not 16000.
    fn rate(&self) -> u32;

    /// How much this source likes to hand over at once - one hardware period
    /// for a capture. Callers size their buffer from it; a `read` given less
    /// than this may refuse.
    fn chunk_frames(&self) -> usize;

    /// Fill `out` and say how many samples were written. Zero means the source
    /// has ended: the file ran out, the limit was reached, or somebody let go
    /// of the button.
    fn read(&mut self, out: &mut [i16]) -> Result<usize>;
}
