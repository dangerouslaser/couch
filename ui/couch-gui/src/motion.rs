//! HA100 lift detection. The kernel exposes raw acceleration in 1024 counts/g.
//! Sampling runs off the render thread and only during display standby. This
//! does not wake a suspended CPU: the board's sensor IRQ wiring is unverified.
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

const SENSOR: &str = "/sys/kernel/couch_motion";

pub struct Controller {
    request: Arc<AtomicU64>,
    quit: Arc<AtomicBool>,
    events: Receiver<u64>,
    generation: u64,
    armed: bool,
}

impl Controller {
    pub fn new() -> Self {
        let request = Arc::new(AtomicU64::new(0));
        let quit = Arc::new(AtomicBool::new(false));
        let (tx, events) = mpsc::sync_channel(1);
        let (worker_request, worker_quit) = (request.clone(), quit.clone());
        thread::spawn(move || {
            let mut generation = 0;
            let mut active = false;
            let mut detector = Detector::default();
            let mut retry = Instant::now();
            let mut failures = 0;
            // Recover an earlier GUI's sensor state after a supervisor restart.
            let _ = std::fs::write(format!("{SENSOR}/enabled"), b"0\n");
            while !worker_quit.load(Ordering::Relaxed) {
                let wanted = worker_request.load(Ordering::Acquire);
                if generation != wanted {
                    if active {
                        let _ = std::fs::write(format!("{SENSOR}/enabled"), b"0\n");
                    }
                    generation = wanted;
                    active = false;
                    detector = Detector::default();
                    failures = 0;
                    retry = Instant::now();
                }
                if generation & 1 == 1 && !active && Instant::now() >= retry {
                    active = std::fs::write(format!("{SENSOR}/enabled"), b"1\n").is_ok();
                    retry = Instant::now() + Duration::from_secs(5);
                }
                if active {
                    let sample = std::fs::read_to_string(format!("{SENSOR}/accel"))
                        .ok()
                        .and_then(|text| parse_sample(&text));
                    if let Some(sample) = sample {
                        failures = 0;
                        if detector.sample(sample) {
                            let _ = tx.try_send(generation);
                            // One notification per arm; don't flood a busy GUI.
                            let _ = std::fs::write(format!("{SENSOR}/enabled"), b"0\n");
                            active = false;
                            retry = Instant::now() + Duration::from_secs(3600);
                        }
                    } else {
                        failures += 1;
                        if failures >= 3 {
                            let _ = std::fs::write(format!("{SENSOR}/enabled"), b"0\n");
                            active = false;
                            detector = Detector::default();
                            retry = Instant::now() + Duration::from_secs(5);
                        }
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
            let _ = std::fs::write(format!("{SENSOR}/enabled"), b"0\n");
        });
        Self {
            request,
            quit,
            events,
            generation: 0,
            armed: false,
        }
    }

    /// Call each GUI loop. Old events are discarded when standby changes.
    pub fn poll(&mut self, armed: bool) -> bool {
        if armed != self.armed {
            self.armed = armed;
            self.generation = self.generation.wrapping_add(2) & !1;
            self.request
                .store(self.generation | u64::from(armed), Ordering::Release);
        }
        let expected = self.generation | u64::from(armed);
        let mut wake = false;
        while let Ok(event) = self.events.try_recv() {
            wake |= armed && event == expected;
        }
        wake
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
    }
}

fn parse_sample(text: &str) -> Option<[i32; 3]> {
    let mut values = text.split_whitespace();
    let sample = [
        values.next()?.parse().ok()?,
        values.next()?.parse().ok()?,
        values.next()?.parse().ok()?,
    ];
    (values.next().is_none() && sample.iter().all(|v| (-2048..=2047).contains(v))).then_some(sample)
}

#[derive(Default)]
struct Detector {
    anchor: Option<[i32; 3]>,
    previous: Option<[i32; 3]>,
    stable: u8,
    moving: u8,
}

fn distance_squared(a: [i32; 3], b: [i32; 3]) -> i32 {
    a.into_iter().zip(b).map(|(a, b)| (a - b).pow(2)).sum()
}

impl Detector {
    fn sample(&mut self, value: [i32; 3]) -> bool {
        // Reject missing/invalid gravity before learning a resting pose.
        let magnitude = distance_squared(value, [0; 3]);
        if !(650 * 650..=1450 * 1450).contains(&magnitude) {
            self.stable = 0;
            self.moving = 0;
            return false;
        }
        if self.anchor.is_none() {
            self.stable = if self
                .previous
                .is_some_and(|p| distance_squared(value, p) < 80 * 80)
            {
                self.stable.saturating_add(1)
            } else {
                0
            };
            self.previous = Some(value);
            if self.stable >= 5 {
                self.anchor = Some(value);
            }
            return false;
        }
        // ~12 degrees of orientation change, sustained for two 100ms samples.
        // A resting baseline and debounce reject table vibration and one-offs.
        if distance_squared(value, self.anchor.unwrap()) >= 210 * 210 {
            self.moving = self.moving.saturating_add(1);
        } else {
            self.moving = 0;
        }
        self.moving >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resting() -> Detector {
        let mut detector = Detector::default();
        for _ in 0..8 {
            assert!(!detector.sample([0, 0, 1024]));
        }
        detector
    }
    #[test]
    fn rejects_noise_and_single_bump_but_detects_lift() {
        let mut detector = resting();
        for n in -30..30 {
            assert!(!detector.sample([n, -n, 1024 + n]));
        }
        assert!(!detector.sample([400, 0, 930]));
        assert!(!detector.sample([0, 0, 1024]));
        assert!(!detector.sample([300, 0, 980]));
        assert!(detector.sample([400, 0, 930]));
    }
    #[test]
    fn waits_for_stable_pose_and_rejects_zero_samples() {
        let mut detector = Detector::default();
        for _ in 0..20 {
            assert!(!detector.sample([0; 3]));
        }
        for i in 0..20 {
            assert!(!detector.sample([if i % 2 == 0 { 400 } else { -400 }, 0, 930]));
        }
        assert!(detector.anchor.is_none());
    }
    #[test]
    fn baseline_is_independent_of_board_orientation() {
        let mut detector = Detector::default();
        for _ in 0..8 {
            assert!(!detector.sample([15, -751, 744]));
        }
        assert!(!detector.sample([30, -430, 930]));
        assert!(detector.sample([50, -300, 975]));
    }
    #[test]
    fn stale_notifications_do_not_wake_a_later_standby_session() {
        let (tx, events) = mpsc::sync_channel(4);
        let mut controller = Controller {
            request: Arc::new(AtomicU64::new(0)),
            quit: Arc::new(AtomicBool::new(false)),
            events,
            generation: 0,
            armed: false,
        };
        assert!(!controller.poll(true));
        let old = controller.request.load(Ordering::Acquire);
        assert!(!controller.poll(false));
        tx.send(old).unwrap();
        assert!(!controller.poll(true));
        let current = controller.request.load(Ordering::Acquire);
        tx.send(current).unwrap();
        assert!(controller.poll(true));
        assert!(!controller.poll(true));
        tx.send(current).unwrap();
        assert!(!controller.poll(false));
    }

    #[test]
    fn parser_rejects_truncated_corrupt_and_out_of_range_samples() {
        assert_eq!(parse_sample("12 -753 778\n"), Some([12, -753, 778]));
        for text in ["1 2", "1 2 3 4", "1 bad 3", "2048 0 0"] {
            assert_eq!(parse_sample(text), None);
        }
    }
}
