//! Explicit, single-write IR bringup probe. Default operation is ioctl-only.
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::time::Instant;

use couch_ir::{abi, pwm};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let zero = args.iter().any(|arg| arg == "--zero");
    let pulse = args.iter().any(|arg| arg == "--pulse");
    if (zero && pulse)
        || args
            .iter()
            .any(|arg| !matches!(arg.as_str(), "--dry-run" | "--zero" | "--pulse"))
    {
        return Err("usage: irtx_probe [--zero | --pulse] [--dry-run]; default: query only".into());
    }
    // One 32-sample word at 228 clocks/sample: 280.62 us. The trailer is
    // separately consumed by Couch's driver and must never be clocked out.
    let words = [if pulse { 0x4924_9249 } else { 0 }, 281u32];
    println!("carrier=38000 sample_clocks=228 sample_us=8.769231");
    println!(
        "operation={} words={words:08x?} waveform_bytes=4 trailer_us=281 dma_us=281",
        if pulse {
            "pulse"
        } else if zero {
            "zero"
        } else {
            "query"
        }
    );
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
    let mut carrier = 38000;
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
