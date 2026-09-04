//! What this CPU actually does in a transformer's inner loop.
//!
//! Whisper's cost is almost all matrix multiply, so "can it run here" reduces
//! to "how many multiply-accumulates per second does this part manage, in the
//! precisions ggml uses". Estimating that from a datasheet is how you get an
//! answer wrong by 3x.
//!
//! Cortex-A7 is in-order, dual-issue, with a 64-bit NEON datapath and no
//! ARMv8.2 dot product - the instruction ggml's fast integer kernels are
//! written around. Its absence is the crux, so int8 is measured the way this
//! core would have to do it: widen, multiply, accumulate.

use std::time::{Duration, Instant};

const N: usize = 128;
const SECONDS: u64 = 3;

fn gemm_f32(a: &[f32], b: &[f32], c: &mut [f32]) {
    for i in 0..N {
        for k in 0..N {
            let av = a[i * N + k];
            let brow = &b[k * N..k * N + N];
            let crow = &mut c[i * N..i * N + N];
            for j in 0..N {
                crow[j] += av * brow[j];
            }
        }
    }
}

fn gemm_i8(a: &[i8], b: &[i8], c: &mut [i32]) {
    for i in 0..N {
        for k in 0..N {
            let av = a[i * N + k] as i32;
            let brow = &b[k * N..k * N + N];
            let crow = &mut c[i * N..i * N + N];
            for j in 0..N {
                crow[j] += av * brow[j] as i32;
            }
        }
    }
}

fn bench<T: Copy + Default + Send + 'static, A: Copy + Default + Send + 'static>(
    threads: usize,
    kernel: fn(&[A], &[A], &mut [T]),
    seed: A,
) -> f64 {
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            std::thread::spawn(move || {
                let a = vec![seed; N * N];
                let b = vec![seed; N * N];
                let mut c = vec![T::default(); N * N];
                let deadline = Instant::now() + Duration::from_secs(SECONDS);
                let mut reps = 0u64;
                while Instant::now() < deadline {
                    kernel(&a, &b, &mut c);
                    reps += 1;
                }
                reps
            })
        })
        .collect();
    let total: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
    // Two flops per multiply-accumulate.
    (total as f64 * 2.0 * (N * N * N) as f64) / (SECONDS as f64) / 1e9
}

fn main() {
    let threads: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    println!("{N}x{N} matmul, {SECONDS}s per case, {threads} thread(s)");
    let f = bench::<f32, f32>(threads, gemm_f32, 1.0);
    println!("  f32   {f:6.2} GFLOP/s");
    let i = bench::<i32, i8>(threads, gemm_i8, 1);
    println!("  int8  {i:6.2} GOP/s");
}

// Measured on the HA100, MT6580 Cortex-A7 at 1.3GHz, four cores under load:
//
//                   1 core        4 cores      4 cores +neon
//   f32             0.14          0.54         0.70 GFLOP/s
//   int8            0.28          1.08         1.91 GOP/s
//
// Two things fall out of that. NEON is not on by default for this target -
// armv7-unknown-linux-musleabihf reports no target features at all - and
// turning it on is worth 77% on integer work. And the ceiling is low enough to
// answer the question this was written for: see docs/voice.md on why Whisper
// does not run here.
//
// It does not help couch-gui, measured four times interleaved: 2009us against
// 2040us per frame, which is noise. The software renderer is bound by how many
// pixels it touches, not by arithmetic - the same conclusion the dirty-region
// work reached from the other direction.
