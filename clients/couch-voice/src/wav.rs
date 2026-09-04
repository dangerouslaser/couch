//! Enough RIFF/WAVE to write what the microphone heard and read it back.
//!
//! A WAV file is the seam that lets the two halves of this crate be tested
//! apart: capture writes one without Home Assistant anywhere near it, and the
//! pipeline client reads one without a microphone. It is also the artefact a
//! probe hands back - a file you can play - which is worth more than any
//! amount of printed statistics when the question is "is the microphone
//! working".
//!
//! Only 16-bit PCM, mono or stereo, because that is what this crate produces
//! and what Home Assistant consumes. A file that is anything else is refused
//! by name rather than misread.

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};

use crate::error::{Error, Result};
use crate::Source;

/// A capture is written as it arrives, so the two length fields are not known
/// until the end. They are patched on `finish`, which means a file left behind
/// by a killed process has zeroes there - readable by this crate, which uses
/// the actual file length, and repaired by simply recording again.
pub struct Writer {
    out: BufWriter<File>,
    path: String,
    frames: u32,
    channels: u16,
    rate: u32,
}

const HEADER_BYTES: u32 = 44;

impl Writer {
    pub fn create(path: &str, rate: u32, channels: u16) -> Result<Writer> {
        let file = File::create(path).map_err(|e| Error::Wav {
            path: path.to_string(),
            detail: format!("cannot create: {e}"),
        })?;
        let mut w = Writer {
            out: BufWriter::new(file),
            path: path.to_string(),
            frames: 0,
            channels,
            rate,
        };
        let header = header(rate, channels, 0);
        w.write_all(&header)?;
        Ok(w)
    }

    pub fn write(&mut self, samples: &[i16]) -> Result<()> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.write_all(&bytes)?;
        self.frames += (samples.len() / self.channels.max(1) as usize) as u32;
        Ok(())
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Patch the sizes and close. Not done in `Drop`, because a failure to
    /// write the header of a recording somebody is waiting for is worth
    /// returning rather than swallowing.
    pub fn finish(mut self) -> Result<()> {
        self.out.flush().map_err(|e| self.fail(e))?;
        let mut file = self.out.into_inner().map_err(|e| Error::Wav {
            path: self.path.clone(),
            detail: format!("cannot flush: {}", e.error()),
        })?;
        file.seek(SeekFrom::Start(0)).map_err(|e| Error::Wav {
            path: self.path.clone(),
            detail: format!("cannot rewind to patch the header: {e}"),
        })?;
        let header = header(self.rate, self.channels, self.frames);
        file.write_all(&header).map_err(|e| Error::Wav {
            path: self.path.clone(),
            detail: format!("cannot write the header: {e}"),
        })?;
        file.sync_all().map_err(|e| Error::Wav {
            path: self.path.clone(),
            detail: format!("cannot sync: {e}"),
        })?;
        Ok(())
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.write_all(bytes).map_err(|e| Error::Wav {
            path: self.path.clone(),
            detail: format!("cannot write: {e}"),
        })
    }

    fn fail(&self, e: io::Error) -> Error {
        Error::Wav {
            path: self.path.clone(),
            detail: format!("cannot write: {e}"),
        }
    }
}

fn header(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
    let block_align = channels * 2;
    let data_bytes = frames * block_align as u32;
    let mut h = Vec::with_capacity(HEADER_BYTES as usize);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(HEADER_BYTES - 8 + data_bytes).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    h.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    h.extend_from_slice(&block_align.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data_bytes.to_le_bytes());
    h
}

/// A whole file in memory, downmixed to mono. Recordings here are seconds
/// long - the capture path caps them - so streaming a WAV off disk would be
/// complexity bought for nothing.
pub struct Reader {
    pub rate: u32,
    pub samples: Vec<i16>,
    at: usize,
    chunk: usize,
}

// Deriving this would print every sample - hundreds of thousands of them -
// the first time anyone put a reader in an assertion.
impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wav::Reader")
            .field("rate", &self.rate)
            .field("frames", &self.samples.len())
            .field("seconds", &self.seconds())
            .finish()
    }
}

impl Reader {
    pub fn open(path: &str) -> Result<Reader> {
        let mut raw = Vec::new();
        File::open(path)
            .and_then(|mut f| f.read_to_end(&mut raw))
            .map_err(|e| Error::Wav {
                path: path.to_string(),
                detail: format!("cannot read: {e}"),
            })?;
        Reader::parse(&raw).map_err(|detail| Error::Wav {
            path: path.to_string(),
            detail,
        })
    }

    /// Walks the chunk list rather than assuming a 44-byte header: anything
    /// that has been through an editor carries LIST or fact chunks, and
    /// reading those as samples produces a burst of noise at the start that
    /// looks like a hardware fault.
    pub fn parse(raw: &[u8]) -> std::result::Result<Reader, String> {
        if raw.len() < 12 || &raw[0..4] != b"RIFF" || &raw[8..12] != b"WAVE" {
            return Err("not a RIFF/WAVE file".into());
        }
        let u16at = |at: usize| u16::from_le_bytes([raw[at], raw[at + 1]]);
        let u32at = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as usize;

        let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
        let mut data: Option<(usize, usize)> = None;
        let mut at = 12;
        while at + 8 <= raw.len() {
            let id = &raw[at..at + 4];
            let size = u32at(at + 4);
            let body = at + 8;
            if id == b"fmt " && body + 16 <= raw.len() {
                let tag = u16at(body);
                // 0xFFFE is WAVE_FORMAT_EXTENSIBLE, whose sub-format is PCM
                // often enough that refusing it outright would be unhelpful.
                if tag != 1 && tag != 0xFFFE {
                    return Err(format!("format tag {tag} is not PCM"));
                }
                channels = u16at(body + 2);
                rate = u32at(body + 4) as u32;
                bits = u16at(body + 14);
            } else if id == b"data" {
                // A file whose header was never patched claims zero bytes; the
                // rest of the file is the recording.
                let end = if size == 0 {
                    raw.len()
                } else {
                    (body + size).min(raw.len())
                };
                data = Some((body, end));
            }
            // Chunks are padded to an even length.
            at = body + size + (size & 1);
        }

        let (start, end) = data.ok_or("no data chunk")?;
        if bits != 16 {
            return Err(format!("{bits}-bit samples; this reads 16-bit PCM only"));
        }
        if channels == 0 || channels > 2 {
            return Err(format!("{channels} channels; this reads mono or stereo"));
        }
        if rate == 0 {
            return Err("no sample rate".into());
        }

        let frames = (end - start) / 2 / channels as usize;
        let mut samples = Vec::with_capacity(frames);
        for f in 0..frames {
            let mut acc = 0i32;
            for c in 0..channels as usize {
                let at = start + (f * channels as usize + c) * 2;
                acc += i16::from_le_bytes([raw[at], raw[at + 1]]) as i32;
            }
            samples.push((acc / channels as i32) as i16);
        }
        Ok(Reader {
            rate,
            samples,
            at: 0,
            chunk: 1024,
        })
    }

    /// A source over samples that are already in hand. Mostly for tests, but
    /// also for anything that has audio from somewhere other than a file.
    pub fn from_samples(samples: Vec<i16>, rate: u32) -> Reader {
        Reader {
            rate,
            samples,
            at: 0,
            chunk: 1024,
        }
    }

    /// How much is handed over per `read`. A live capture's chunk is one
    /// hardware period; a file has no natural size, so callers streaming to
    /// something that cares can pick one.
    pub fn with_chunk(mut self, frames: usize) -> Reader {
        self.chunk = frames.max(1);
        self
    }

    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / self.rate.max(1) as f32
    }
}

impl Source for Reader {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn chunk_frames(&self) -> usize {
        self.chunk
    }

    fn read(&mut self, out: &mut [i16]) -> Result<usize> {
        let n = self.samples.len().saturating_sub(self.at).min(out.len());
        out[..n].copy_from_slice(&self.samples[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_reads_back_as_what_went_in() {
        let dir = std::env::temp_dir().join("couch-voice-wav-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("round-trip.wav");
        let path = path.to_str().unwrap();

        let samples: Vec<i16> = (0..4000)
            .map(|i| ((i * 37) % 20000) as i16 - 10000)
            .collect();
        let mut w = Writer::create(path, 16000, 1).unwrap();
        for chunk in samples.chunks(333) {
            w.write(chunk).unwrap();
        }
        assert_eq!(w.frames(), 4000);
        w.finish().unwrap();

        let r = Reader::open(path).unwrap();
        assert_eq!(r.rate, 16000);
        assert_eq!(r.samples, samples);
        assert!((r.seconds() - 0.25).abs() < 0.001);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn stereo_is_folded_to_mono() {
        let mut raw = header(16000, 2, 2);
        for s in [1000i16, 2000, -400, -600] {
            raw.extend_from_slice(&s.to_le_bytes());
        }
        let r = Reader::parse(&raw).unwrap();
        assert_eq!(r.samples, vec![1500, -500]);
    }

    /// The bug this guards: a file with a LIST chunk before the data, read as
    /// if the header were always 44 bytes, starts with a burst of garbage that
    /// is easy to blame on the microphone.
    #[test]
    fn chunks_before_the_data_are_skipped() {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        raw.extend_from_slice(b"LIST");
        raw.extend_from_slice(&5u32.to_le_bytes());
        raw.extend_from_slice(b"INFOx");
        raw.push(0); // pad to an even length
        raw.extend_from_slice(b"fmt ");
        raw.extend_from_slice(&16u32.to_le_bytes());
        raw.extend_from_slice(&1u16.to_le_bytes());
        raw.extend_from_slice(&1u16.to_le_bytes());
        raw.extend_from_slice(&16000u32.to_le_bytes());
        raw.extend_from_slice(&32000u32.to_le_bytes());
        raw.extend_from_slice(&2u16.to_le_bytes());
        raw.extend_from_slice(&16u16.to_le_bytes());
        raw.extend_from_slice(b"data");
        raw.extend_from_slice(&4u32.to_le_bytes());
        raw.extend_from_slice(&77i16.to_le_bytes());
        raw.extend_from_slice(&(-77i16).to_le_bytes());
        let r = Reader::parse(&raw).unwrap();
        assert_eq!(r.samples, vec![77, -77]);
    }

    #[test]
    fn a_truncated_header_is_refused_by_name() {
        assert!(Reader::parse(b"not a wav at all").is_err());
        let mut raw = header(16000, 1, 0);
        raw[34] = 24; // bits per sample
        let e = Reader::parse(&raw).unwrap_err();
        assert!(e.contains("24-bit"), "{e}");
    }

    #[test]
    fn a_file_whose_length_was_never_patched_still_reads() {
        let mut raw = header(16000, 1, 0);
        for s in [5i16, 6, 7] {
            raw.extend_from_slice(&s.to_le_bytes());
        }
        assert_eq!(Reader::parse(&raw).unwrap().samples, vec![5, 6, 7]);
    }
}
