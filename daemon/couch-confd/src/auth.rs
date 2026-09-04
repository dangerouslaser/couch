//! Pairing by a PIN shown on the remote's own screen.
//!
//! The credential is physical access to the remote, which is the same boundary
//! the SSH enrolment button already draws: if you can read the screen, you are
//! standing in the house. Nothing is stored, nothing is enrolled ahead of time,
//! and there is no password for anyone to forget or write down.
//!
//! Four digits is only ten thousand guesses, so the digits alone are not the
//! defence - the defence is that a guess costs an attempt against a challenge
//! that expires, and burning through the attempts is visible. A PIN lives two
//! minutes, dies after five wrong answers, and every new challenge lights up
//! the remote. An attacker on the LAN who wants to brute force this has to make
//! the remote in your living room flash a fresh PIN a thousand times.
//!
//! That visibility is the quiet second feature: a PIN appearing when nobody
//! opened the page means somebody else on the network just tried.
//!
//! Sessions live in memory only. Restarting the daemon logs everyone out, which
//! is the right way round - a config daemon that survives a reboot holding open
//! sessions is one that cannot be reset by turning it off and on again.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Long enough to walk to the remote and read it, short enough that a PIN left
/// on screen because someone wandered off is not a standing invitation.
const PIN_TTL: Duration = Duration::from_secs(120);

/// Wrong answers before the challenge is destroyed. Five is generous for
/// someone squinting at a small screen and nowhere near enough to search a
/// four-digit space.
const MAX_TRIES: u8 = 5;

/// A paired browser stays paired for a week of use. This is a tool you open
/// twice a month; re-pairing every time would push people towards leaving auth
/// off, which is the outcome this whole module exists to avoid.
const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Where the running GUI looks for a PIN to display. `/tmp` is shared: the
/// daemon and `couch-gui` are both inside the Alpine root, which is how
/// `/tmp/couch.setup` and `/tmp/portal.ssid` already pass between the portal
/// and the GUI.
pub const DEFAULT_PIN_FILE: &str = "/tmp/couch.pin";

pub const COOKIE: &str = "couch_session";

pub struct Auth {
    state: Mutex<State>,
    pin_file: PathBuf,
    /// Set by `--no-auth`, for a laptop with no remote attached to read a PIN
    /// off. Everything below still runs; the gate simply always opens.
    disabled: bool,
}

struct State {
    challenge: Option<Challenge>,
    /// token -> when it was last used. Idle expiry rather than absolute, so the
    /// phone you actually use does not log itself out mid-edit.
    sessions: HashMap<String, Instant>,
}

struct Challenge {
    pin: String,
    started: Instant,
    tries: u8,
}

/// What `GET /api/auth/status` tells a browser that has not paired yet. It
/// deliberately does not include the PIN: the whole point is that the digits
/// exist only on the remote's screen.
pub struct Status {
    pub authenticated: bool,
    pub pairing: bool,
    pub expires_in: u64,
    pub tries_left: u8,
    pub disabled: bool,
}

pub enum Verdict {
    /// Paired. The token goes back as a cookie.
    Paired(String),
    Wrong { tries_left: u8 },
    /// No challenge, or it aged out. The browser should ask for a new one,
    /// which lights the remote up again.
    Expired,
}

impl Auth {
    pub fn new(pin_file: impl Into<PathBuf>, disabled: bool) -> Auth {
        let auth = Auth {
            state: Mutex::new(State { challenge: None, sessions: HashMap::new() }),
            pin_file: pin_file.into(),
            disabled,
        };
        // A PIN file left by a killed daemon would have the remote showing
        // digits that no longer open anything.
        auth.clear_pin_file();
        auth
    }

    pub fn disabled(&self) -> bool {
        self.disabled
    }

    /// True if this request carries a live session. Also refreshes it, which is
    /// what makes the timeout idle-based.
    pub fn authenticated(&self, cookies: &str) -> bool {
        if self.disabled {
            return true;
        }
        let Some(token) = cookie_value(cookies, COOKIE) else { return false };
        let mut state = self.state.lock().unwrap();
        state.sessions.retain(|_, seen| seen.elapsed() < SESSION_TTL);
        match state.sessions.get_mut(&token) {
            Some(seen) => {
                *seen = Instant::now();
                true
            }
            None => false,
        }
    }

    /// Start a challenge, or report on the one already running. Asking twice
    /// does not reroll the digits: a browser that polls, or a second tab, must
    /// not change what the remote is showing halfway through someone typing it.
    pub fn challenge(&self) -> Status {
        let mut state = self.state.lock().unwrap();
        self.expire_locked(&mut state);
        if state.challenge.is_none() {
            let pin = random_pin();
            self.write_pin_file(&pin);
            state.challenge = Some(Challenge { pin, started: Instant::now(), tries: 0 });
        }
        self.status_locked(&state, false)
    }

    pub fn status(&self, cookies: &str) -> Status {
        let authenticated = self.authenticated(cookies);
        let mut state = self.state.lock().unwrap();
        self.expire_locked(&mut state);
        self.status_locked(&state, authenticated)
    }

    pub fn verify(&self, offered: &str) -> Verdict {
        let mut state = self.state.lock().unwrap();
        self.expire_locked(&mut state);
        let Some(challenge) = state.challenge.as_mut() else { return Verdict::Expired };

        if constant_time_eq(challenge.pin.as_bytes(), offered.trim().as_bytes()) {
            state.challenge = None;
            self.clear_pin_file();
            let token = random_token();
            state.sessions.insert(token.clone(), Instant::now());
            return Verdict::Paired(token);
        }

        challenge.tries += 1;
        let tries_left = MAX_TRIES.saturating_sub(challenge.tries);
        if tries_left == 0 {
            // Destroying the challenge rather than just refusing is the point:
            // the next attempt needs a fresh PIN, and a fresh PIN is another
            // prompt on the remote that somebody has to be standing next to.
            state.challenge = None;
            self.clear_pin_file();
            return Verdict::Expired;
        }
        Verdict::Wrong { tries_left }
    }

    pub fn log_out(&self, cookies: &str) {
        if let Some(token) = cookie_value(cookies, COOKIE) {
            self.state.lock().unwrap().sessions.remove(&token);
        }
    }

    fn status_locked(&self, state: &State, authenticated: bool) -> Status {
        match &state.challenge {
            Some(c) => Status {
                authenticated,
                pairing: true,
                expires_in: PIN_TTL.saturating_sub(c.started.elapsed()).as_secs(),
                tries_left: MAX_TRIES.saturating_sub(c.tries),
                disabled: self.disabled,
            },
            None => Status {
                authenticated,
                pairing: false,
                expires_in: 0,
                tries_left: MAX_TRIES,
                disabled: self.disabled,
            },
        }
    }

    /// Expiry is checked when something asks rather than on a timer, so the
    /// daemon has no thread doing nothing. The consequence is that the file can
    /// outlive the challenge if no request ever arrives, which is why the GUI
    /// runs its own countdown over what it reads rather than trusting the file
    /// to vanish.
    fn expire_locked(&self, state: &mut State) {
        let stale = state.challenge.as_ref().is_some_and(|c| c.started.elapsed() >= PIN_TTL);
        if stale {
            state.challenge = None;
            self.clear_pin_file();
        }
    }

    fn write_pin_file(&self, pin: &str) {
        // 0600: anything that can read this can pair, so it is a credential for
        // as long as it exists.
        let _ = write_private(&self.pin_file, pin.as_bytes());
    }

    fn clear_pin_file(&self) {
        let _ = std::fs::remove_file(&self.pin_file);
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// `Cookie: a=1; couch_session=deadbeef`
fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_string())
    })
}

/// A comparison whose duration does not depend on how much of the PIN was
/// right. The remote timing signal here is small, but a four-digit secret is
/// small too, and this costs four lines.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_pin() -> String {
    // Modulo bias over a u64 is about one part in 10^15, which is not the weak
    // point of a four-digit PIN.
    format!("{:04}", random_u64() % 10_000)
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    fill_random(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_u64() -> u64 {
    let mut bytes = [0u8; 8];
    fill_random(&mut bytes);
    u64::from_le_bytes(bytes)
}

/// `/dev/urandom` rather than a crate: it is the same source `getrandom` would
/// reach for on both targets this builds for, and it is one file open against
/// a dependency tree.
///
/// There is no fallback to a weaker source on purpose. A PIN from a
/// clock-seeded PRNG looks exactly like a real one while being guessable, so if
/// the kernel cannot give us entropy the honest move is to stop.
fn fill_random(out: &mut [u8]) {
    let mut file = std::fs::File::open("/dev/urandom")
        .expect("cannot open /dev/urandom - refusing to invent a PIN");
    file.read_exact(out).expect("cannot read /dev/urandom - refusing to invent a PIN");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> (Auth, PathBuf) {
        let path = std::env::temp_dir().join(format!("couch-pin-test-{}", random_u64()));
        (Auth::new(path.clone(), false), path)
    }

    #[test]
    fn a_pin_reaches_the_screen_and_opens_the_gate() {
        let (auth, path) = auth();
        assert!(!auth.authenticated(""));

        auth.challenge();
        let shown = std::fs::read_to_string(&path).unwrap();
        assert_eq!(shown.len(), 4);
        assert!(shown.chars().all(|c| c.is_ascii_digit()));

        let Verdict::Paired(token) = auth.verify(&shown) else { panic!("should have paired") };
        assert!(auth.authenticated(&format!("{COOKIE}={token}")));
        // The digits stop being a credential the moment they are spent.
        assert!(!path.exists());
    }

    #[test]
    fn a_wrong_pin_burns_an_attempt_and_five_burn_the_challenge() {
        let (auth, path) = auth();
        auth.challenge();
        let right = std::fs::read_to_string(&path).unwrap();
        let wrong = format!("{:04}", (right.parse::<u32>().unwrap() + 1) % 10_000);

        for expected in (1..MAX_TRIES).rev() {
            match auth.verify(&wrong) {
                Verdict::Wrong { tries_left } => assert_eq!(tries_left, expected),
                _ => panic!("should have counted a wrong answer"),
            }
        }
        assert!(matches!(auth.verify(&wrong), Verdict::Expired));
        // Even the right answer is no good now - the challenge is gone.
        assert!(matches!(auth.verify(&right), Verdict::Expired));
        assert!(!path.exists());
    }

    #[test]
    fn asking_twice_does_not_move_the_digits_underneath_someone() {
        let (auth, path) = auth();
        auth.challenge();
        let first = std::fs::read_to_string(&path).unwrap();
        auth.challenge();
        assert_eq!(first, std::fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn logging_out_ends_that_session_only() {
        let (auth, path) = auth();
        auth.challenge();
        let pin = std::fs::read_to_string(&path).unwrap();
        let Verdict::Paired(one) = auth.verify(&pin) else { panic!() };

        auth.challenge();
        let pin = std::fs::read_to_string(&path).unwrap();
        let Verdict::Paired(two) = auth.verify(&pin) else { panic!() };

        auth.log_out(&format!("{COOKIE}={one}"));
        assert!(!auth.authenticated(&format!("{COOKIE}={one}")));
        assert!(auth.authenticated(&format!("{COOKIE}={two}")));
    }

    #[test]
    fn no_auth_opens_everything() {
        let path = std::env::temp_dir().join(format!("couch-pin-test-{}", random_u64()));
        let auth = Auth::new(path, true);
        assert!(auth.authenticated(""));
    }

    #[test]
    fn cookies_are_read_out_of_a_crowded_header() {
        assert_eq!(cookie_value("a=1; couch_session=abc; b=2", COOKIE).unwrap(), "abc");
        assert_eq!(cookie_value("couch_session=abc", COOKIE).unwrap(), "abc");
        assert!(cookie_value("a=1; b=2", COOKIE).is_none());
        // A cookie whose name merely ends in ours is a different cookie.
        assert!(cookie_value("not_couch_session=abc", COOKIE).is_none());
    }
}
