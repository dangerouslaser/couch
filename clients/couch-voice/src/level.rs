//! Two questions about a block of samples, and they are not the same question.
//!
//! A *level* is what a UI draws while recording: peak and RMS of the last
//! chunk, cheap enough to compute on every period.
//!
//! An *analysis* is what a probe prints afterwards, and it exists because
//! "did that record anything" has four answers that all look identical in a
//! waveform thumbnail: nothing at all, a stuck DC value, the noise floor of a
//! microphone that is powered but disconnected, and actual speech. Telling
//! them apart needs the statistics below - variation *over time* being the one
//! that matters, because a hiss and a voice have similar RMS and completely
//! different envelopes.

/// Peak and RMS of one chunk, as amplitude in 0..1 and in dBFS.
#[derive(Debug, Clone, Copy, Default)]
pub struct Level {
    pub peak: f32,
    pub rms: f32,
}

/// Quiet enough that calling it silence is safe: 16-bit dither sits around
/// -90 dBFS and any real signal is far above this.
pub const SILENCE_DBFS: f32 = -80.0;

impl Level {
    pub fn peak_dbfs(&self) -> f32 {
        dbfs(self.peak)
    }

    pub fn rms_dbfs(&self) -> f32 {
        dbfs(self.rms)
    }

    /// 0..1 for a meter, mapping -60 dBFS to the bottom of the scale. Linear
    /// amplitude spends its whole range in the top of a bar and looks dead.
    pub fn meter(&self) -> f32 {
        ((self.peak_dbfs() + 60.0) / 60.0).clamp(0.0, 1.0)
    }
}

pub fn dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * amplitude.log10()
    }
}

/// Keeps the level of the most recent chunk.
#[derive(Debug, Default)]
pub struct Meter {
    level: Level,
}

impl Meter {
    pub fn feed(&mut self, samples: &[i16]) {
        self.level = level_of(samples);
    }

    pub fn level(&self) -> Level {
        self.level
    }
}

pub fn level_of(samples: &[i16]) -> Level {
    if samples.is_empty() {
        return Level::default();
    }
    let mut peak = 0i32;
    let mut sum = 0f64;
    for s in samples {
        let v = *s as i32;
        peak = peak.max(v.abs());
        sum += (v as f64) * (v as f64);
    }
    let scale = i16::MAX as f64;
    Level {
        peak: (peak as f64 / scale) as f32,
        rms: ((sum / samples.len() as f64).sqrt() / scale) as f32,
    }
}

/// What a recording turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every sample identical. A device that is clocked but whose converter is
    /// off, or a buffer that was never written.
    Dead,
    /// Moving, but below the noise floor of anything real.
    Silent,
    /// A steady hiss with no envelope. The microphone is powered and hearing
    /// nothing - or is not connected to a capsule.
    NoiseOnly,
    /// Loud, and clipping. Real, but the gain is too high to transcribe.
    Clipping,
    /// Something happened, and it had a shape.
    Signal,
}

impl Verdict {
    pub fn line(&self) -> &'static str {
        match self {
            Verdict::Dead => "DEAD      - every sample is the same value; nothing is converting",
            Verdict::Silent => "SILENT    - below the noise floor; no audible input at all",
            Verdict::NoiseOnly => {
                "NOISE     - a steady floor with no envelope; powered, hearing nothing"
            }
            Verdict::Clipping => "CLIPPING  - real audio, but too loud to transcribe; cut the gain",
            Verdict::Signal => "SIGNAL    - audio with a shape; this device is hearing something",
        }
    }
}

/// Everything a probe needs to say what a recording was.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub frames: usize,
    pub rate: u32,
    pub peak: f32,
    pub rms: f32,
    /// Mean sample value as a fraction of full scale. A capsule sitting on a
    /// bias voltage shows up here and nowhere else.
    pub dc_offset: f32,
    pub min: i16,
    pub max: i16,
    /// Distinct sample values, counted up to a cap. One means dead; a handful
    /// means a converter stuck on its lowest bits.
    pub distinct: usize,
    pub zeros: usize,
    /// RMS in dBFS per 100 ms window - the envelope, and the thing that
    /// separates speech from hiss.
    pub windows: Vec<f32>,
    pub clipped: usize,
    pub verdict: Verdict,
}

/// Windows short enough to catch a syllable and long enough to be stable.
const WINDOW_MS: usize = 100;

/// Anything within one LSB of full scale is counted as clipped. Compared
/// against the unsigned magnitude, because `i16::MIN.abs()` overflows and a
/// rail-to-rail signal is exactly where that would happen.
const CLIP: u16 = i16::MAX as u16 - 1;

/// How much the loudest 100 ms has to beat the quietest before a recording is
/// speech rather than a hiss. Room tone varies by a couple of dB; a voice
/// against room tone is tens.
const ENVELOPE_DB: f32 = 12.0;

pub fn analyse(samples: &[i16], rate: u32) -> Analysis {
    let level = level_of(samples);
    let mut distinct = std::collections::HashSet::new();
    let mut zeros = 0usize;
    let mut clipped = 0usize;
    let mut sum = 0f64;
    let (mut min, mut max) = (i16::MAX, i16::MIN);
    for s in samples {
        if distinct.len() < 4096 {
            distinct.insert(*s);
        }
        if *s == 0 {
            zeros += 1;
        }
        if s.unsigned_abs() >= CLIP {
            clipped += 1;
        }
        sum += *s as f64;
        min = min.min(*s);
        max = max.max(*s);
    }
    let n = samples.len().max(1);
    let window = (rate as usize * WINDOW_MS / 1000).max(1);
    let windows: Vec<f32> = samples
        .chunks(window)
        // A trailing part-window is not comparable with the full ones, and on
        // a short recording it can be the quietest by arithmetic alone.
        .filter(|c| c.len() == window)
        .map(|c| level_of(c).rms_dbfs())
        .collect();

    let loudest = windows.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let quietest = windows.iter().cloned().fold(f32::INFINITY, f32::min);
    let envelope = if windows.len() < 2 {
        0.0
    } else {
        loudest - quietest
    };

    let verdict = if distinct.len() <= 1 {
        Verdict::Dead
    } else if level.peak_dbfs() < SILENCE_DBFS {
        Verdict::Silent
    } else if clipped * 100 > n {
        // More than one sample in a hundred pinned to the rail is not a drum
        // hit, it is a gain problem.
        Verdict::Clipping
    } else if envelope >= ENVELOPE_DB && level.peak_dbfs() > -45.0 {
        Verdict::Signal
    } else {
        Verdict::NoiseOnly
    };

    Analysis {
        frames: samples.len(),
        rate,
        peak: level.peak,
        rms: level.rms,
        dc_offset: (sum / n as f64 / i16::MAX as f64) as f32,
        min,
        max,
        distinct: distinct.len(),
        zeros,
        windows,
        clipped,
        verdict,
    }
}

impl Analysis {
    pub fn peak_dbfs(&self) -> f32 {
        dbfs(self.peak)
    }

    pub fn rms_dbfs(&self) -> f32 {
        dbfs(self.rms)
    }

    pub fn seconds(&self) -> f32 {
        self.frames as f32 / self.rate.max(1) as f32
    }

    /// The envelope as an ASCII bar per 100 ms, so a recording can be read
    /// over a serial console with no tooling at all.
    pub fn sparkline(&self) -> String {
        const BLOCKS: &[u8] = b"_.:-=+*#%@";
        self.windows
            .iter()
            .map(|db| {
                let t = ((db + 70.0) / 70.0).clamp(0.0, 0.999);
                BLOCKS[(t * BLOCKS.len() as f32) as usize] as char
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, secs: f32, amp: f32, envelope: bool) -> Vec<i16> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                // A gate that is open for a third of the recording, which is
                // roughly what a two-word command looks like.
                let gain = if envelope && !(0.3..0.6).contains(&(t / secs)) {
                    0.002
                } else {
                    1.0
                };
                let v = (t * 440.0 * std::f32::consts::TAU).sin() * amp * gain;
                (v * i16::MAX as f32) as i16
            })
            .collect()
    }

    #[test]
    fn a_flat_buffer_is_dead_not_silent() {
        let a = analyse(&[0; 16000], 16000);
        assert_eq!(a.verdict, Verdict::Dead);
        assert_eq!(a.distinct, 1);
        // A converter stuck on a non-zero value is the same fault.
        let b = analyse(&[-1234; 16000], 16000);
        assert_eq!(b.verdict, Verdict::Dead);
    }

    #[test]
    fn a_hiss_is_not_a_voice() {
        // Deterministic pseudo-noise, constant level throughout.
        let mut x = 12345u32;
        let noise: Vec<i16> = (0..16000)
            .map(|_| {
                x = x.wrapping_mul(1103515245).wrapping_add(12345);
                ((x >> 20) as i16 % 600) - 300
            })
            .collect();
        let a = analyse(&noise, 16000);
        assert_eq!(a.verdict, Verdict::NoiseOnly);
        assert!(a.distinct > 100);
    }

    #[test]
    fn speech_shaped_audio_reads_as_signal() {
        let a = analyse(&tone(16000, 2.0, 0.4, true), 16000);
        assert_eq!(a.verdict, Verdict::Signal);
        assert!(a.sparkline().len() == a.windows.len());
    }

    #[test]
    fn a_rail_to_rail_signal_is_reported_as_clipping() {
        let a = analyse(&tone(16000, 1.0, 4.0, false), 16000);
        assert_eq!(a.verdict, Verdict::Clipping);
    }

    /// Below -80 dBFS but still moving: a couple of counts of dither, which
    /// is what a live-but-mute converter looks like. Not `Dead`, which needs
    /// the samples to be identical.
    #[test]
    fn very_quiet_is_silent_rather_than_noise() {
        let a = analyse(&tone(16000, 1.0, 0.00008, false), 16000);
        assert!(a.distinct > 1, "{a:?}");
        assert_eq!(a.verdict, Verdict::Silent);
    }

    #[test]
    fn levels_are_in_dbfs() {
        let full = level_of(&[i16::MAX, i16::MIN + 1]);
        assert!((full.peak_dbfs() - 0.0).abs() < 0.01);
        assert_eq!(level_of(&[0, 0]).peak_dbfs(), f32::NEG_INFINITY);
        assert_eq!(level_of(&[]).meter(), 0.0);
    }
}
