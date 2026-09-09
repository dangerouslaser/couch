//! Explicit, single-write IR bringup probe. Default operation is ioctl-only.
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::time::Instant;

use couch_ir::{abi, pwm};

fn carrier_clocks(carrier: u32) -> Result<u32, &'static str> {
    if !(10_000..=100_000).contains(&carrier) {
        return Err("--carrier must be between 10000 and 100000 Hz");
    }
    let denominator = carrier * 3;
    Ok((26_000_000 + denominator / 2) / denominator)
}

fn waveform(zero_us: Option<u32>, pulse: bool, carrier: u32) -> Result<Vec<u32>, &'static str> {
    let clocks = u64::from(carrier_clocks(carrier)?);
    let clocks_per_word = clocks * 32;
    let (word_count, trailer) = if let Some(us) = zero_us {
        if !(1..=1_000_000).contains(&us) {
            return Err("--zero-us must be between 1 and 1000000");
        }
        (
            ((u64::from(us) * 26 + clocks_per_word - 1) / clocks_per_word) as usize,
            us,
        )
    } else {
        (1, ((clocks_per_word + 25) / 26) as u32)
    };
    let mut words = vec![0; word_count + 1];
    if pulse {
        words[0] = 0x4924_9249;
    }
    words[word_count] = trailer;
    Ok(words)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut dry_run = false;
    let mut zero = false;
    let mut pulse = false;
    let mut zero_us = None;
    let mut carrier = 38000;
    let mut carrier_seen = false;
    let mut modes = 0;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--carrier" => {
                if carrier_seen {
                    return Err("--carrier supplied more than once".into());
                }
                carrier = args.next().ok_or("--carrier needs Hz")?.parse::<u32>()?;
                carrier_seen = true;
            }
            "--zero" => {
                zero = true;
                modes += 1;
            }
            "--pulse" => {
                pulse = true;
                modes += 1;
            }
            "--zero-us" => {
                zero_us = Some(
                    args.next()
                        .ok_or("--zero-us needs microseconds")?
                        .parse::<u32>()?,
                );
                zero = true;
                modes += 1;
            }
            _ => {
                return Err(
                    "usage: irtx_probe [--zero | --pulse | --zero-us MICROSECONDS] [--dry-run]"
                        .into(),
                )
            }
        }
    }
    if modes > 1 {
        return Err("choose exactly one waveform mode".into());
    }
    let clocks = carrier_clocks(carrier)?;
    let words = waveform(zero_us, pulse, carrier)?;
    let wave_bytes = (words.len() - 1) * 4;
    let expected_us = (wave_bytes as u64 * 8 * u64::from(clocks) + 25) / 26;
    println!(
        "carrier={carrier} sample_clocks={clocks} sample_us={:.6}",
        f64::from(clocks) / 26.0
    );
    println!("operation={} waveform_words={} first_word={:08x} waveform_bytes={} trailer_us={} dma_us={}",
        if pulse { "pulse" } else if zero { "zero" } else { "query" },
        words.len() - 1, words[0], wave_bytes, words[words.len() - 1], expected_us);
    if dry_run {
        return Ok(());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/irtx")?;
    let mut solution = 0;
    abi::ioctl_u32(
        file.as_raw_fd(),
        abi::IRTX_IOC_GET_SOLUTTION_TYPE,
        &mut solution,
    )?;
    println!("solution={solution}");
    if solution != 1 {
        return Err("expected Couch PWM-only solution 1; no write attempted".into());
    }
    abi::ioctl_u32(
        file.as_raw_fd(),
        abi::IRTX_IOC_SET_CARRIER_FREQ,
        &mut carrier,
    )?;
    println!("carrier ioctl accepted");
    if !zero && !pulse {
        return Ok(());
    }
    let bytes = pwm::to_bytes(&words);
    println!(
        "write_begin bytes={}; exactly one syscall, no retry",
        bytes.len()
    );
    io::stdout().flush()?;
    let start = Instant::now();
    let written = unsafe { libc::write(file.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
    let error = if written < 0 {
        Some(io::Error::last_os_error())
    } else {
        None
    };
    println!(
        "write_return bytes={written} elapsed_us={} error={error:?}",
        start.elapsed().as_micros()
    );
    if let Some(error) = error {
        return Err(error.into());
    }
    if written as usize != bytes.len() {
        return Err("short write; deliberately not retrying the remainder".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_matrix_has_no_marks_and_keeps_duration_outside_dma() {
        for (us, count, expected_us) in [
            (68000, 243, 68190),
            (500000, 1782, 500057),
            (1000000, 3564, 1000114),
        ] {
            let words = waveform(Some(us), false, 38000).unwrap();
            assert_eq!(words.len(), count + 1);
            assert!(words[..count].iter().all(|word| *word == 0));
            assert_eq!(words[count], us);
            assert_eq!((count as u64 * 32 * 228 + 25) / 26, expected_us);
            assert!(words.len() * 4 <= 128 * 1024);
        }
    }

    #[test]
    fn invalid_durations_are_rejected_before_opening_device() {
        assert!(waveform(Some(0), false, 38000).is_err());
        assert!(waveform(Some(1_000_001), false, 38000).is_err());
        assert!(waveform(Some(u32::MAX), false, 38000).is_err());
    }

    #[test]
    fn original_probes_keep_exact_single_word_payloads() {
        assert_eq!(waveform(None, false, 38000).unwrap(), [0, 281]);
        assert_eq!(waveform(None, true, 38000).unwrap(), [0x4924_9249, 281]);
    }
    #[test]
    fn carrier_controls_zero_wave_geometry_and_bounds() {
        for (carrier, clocks) in [(36000, 241), (38000, 228), (40000, 217)] {
            assert_eq!(carrier_clocks(carrier).unwrap(), clocks);
            let words = waveform(Some(68000), false, carrier).unwrap();
            let ticks = (words.len() - 1) as u64 * 32;
            let duration_clocks = ticks * u64::from(clocks);
            assert!(duration_clocks >= 68000 * 26);
            assert!(duration_clocks < 68000 * 26 + u64::from(clocks) * 32);
            assert!(words[..words.len() - 1].iter().all(|word| *word == 0));
            assert_eq!(words.last(), Some(&68000));
        }
        for carrier in [0, 9999, 100001, u32::MAX] {
            assert!(carrier_clocks(carrier).is_err());
            assert!(waveform(None, false, carrier).is_err());
        }
        assert!(carrier_clocks(10000).is_ok());
        assert!(carrier_clocks(100000).is_ok());
    }
}
