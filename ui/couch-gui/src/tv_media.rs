//! Optional Cast now-playing observer. Its network and image workers never gate keys.
use crate::App;
use couch_androidtv::cast::{Observer, Status};
use std::{
    io::{Cursor, Read},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default, PartialEq)]
struct Target {
    generation: u64,
    connection: String,
}
struct Snapshot {
    generation: u64,
    status: Status,
    at: Instant,
}
#[derive(Clone, PartialEq)]
struct ArtKey {
    generation: u64,
    session: i64,
    url: String,
}
struct Pixels {
    width: u32,
    height: u32,
    data: Vec<u8>,
}
struct ArtReply {
    key: ArtKey,
    pixels: Option<Pixels>,
}

pub(super) struct Controller {
    target: Arc<Mutex<Target>>,
    latest: Arc<Mutex<Option<Snapshot>>>,
    art_target: Arc<Mutex<Option<ArtKey>>>,
    art_reply: Arc<Mutex<Option<ArtReply>>>,
    art_key: Option<ArtKey>,
    generation: u64,
}
impl Controller {
    pub fn new() -> Self {
        let target = Arc::new(Mutex::new(Target::default()));
        let latest = Arc::new(Mutex::new(None));
        let art_target = Arc::new(Mutex::new(None));
        let art_reply = Arc::new(Mutex::new(None));
        let (t, l) = (target.clone(), latest.clone());
        std::thread::spawn(move || observe(t, l));
        let (t, r) = (art_target.clone(), art_reply.clone());
        std::thread::spawn(move || artwork(t, r));
        Self {
            target,
            latest,
            art_target,
            art_reply,
            art_key: None,
            generation: 0,
        }
    }
    pub fn open(&mut self, app: &App, generation: u64, connection: &str) {
        self.clear(app);
        self.generation = generation;
        *self.target.lock().unwrap() = Target {
            generation,
            connection: connection.into(),
        };
    }
    pub fn clear(&mut self, app: &App) {
        self.generation = 0;
        *self.target.lock().unwrap() = Target::default();
        *self.latest.lock().unwrap() = None;
        *self.art_target.lock().unwrap() = None;
        self.art_key = None;
        app.set_tv_media_active(false);
        app.set_tv_media_has_art(false);
        app.set_tv_media_art(slint::Image::default());
    }
    pub fn poll(&mut self, app: &App) {
        if !app.get_tv_shown() || !app.get_tv_android() {
            if self.generation != 0 {
                self.clear(app);
            }
            return;
        }
        let latest = self.latest.lock().unwrap();
        let media = latest
            .as_ref()
            .filter(|s| s.generation == self.generation && s.at.elapsed() < Duration::from_secs(3))
            .and_then(|s| s.status.now_playing.as_ref());
        if let Some(media) = media {
            app.set_tv_media_active(true);
            app.set_tv_media_app(
                latest
                    .as_ref()
                    .and_then(|s| s.status.app_name.as_deref())
                    .unwrap_or("Android TV")
                    .into(),
            );
            app.set_tv_media_title(media.title.as_str().into());
            app.set_tv_media_subtitle(media.subtitle.as_str().into());
            app.set_tv_media_state(
                match media.player_state.as_str() {
                    "PLAYING" => "Playing",
                    "PAUSED" => "Paused",
                    "BUFFERING" => "Buffering",
                    _ => "",
                }
                .into(),
            );
            app.set_tv_media_live(media.live);
            let (position, duration, progress) =
                timeline(media.position, media.duration, media.live);
            app.set_tv_media_position(if media.position_known {
                position.into()
            } else {
                "—".into()
            });
            app.set_tv_media_has_duration(duration.is_some() && media.position_known);
            app.set_tv_media_duration(duration.unwrap_or_default().into());
            app.set_tv_media_progress(progress);
            let key = media.artwork_url.as_ref().map(|url| ArtKey {
                generation: self.generation,
                session: media.session_id,
                url: url.clone(),
            });
            if key != self.art_key {
                app.set_tv_media_has_art(false);
                app.set_tv_media_art(slint::Image::default());
                *self.art_target.lock().unwrap() = key.clone();
                self.art_key = key;
            }
        } else {
            app.set_tv_media_active(false);
            app.set_tv_media_has_art(false);
            app.set_tv_media_art(slint::Image::default());
            self.art_key = None;
            *self.art_target.lock().unwrap() = None;
        }
        drop(latest);
        if let Some(reply) = self.art_reply.lock().unwrap().take() {
            if Some(&reply.key) == self.art_key.as_ref() {
                if let Some(p) = reply.pixels {
                    app.set_tv_media_art(slint::Image::from_rgba8(slint::SharedPixelBuffer::<
                        slint::Rgba8Pixel,
                    >::clone_from_slice(
                        &p.data, p.width, p.height
                    )));
                    app.set_tv_media_has_art(true);
                }
            }
        }
    }
}
fn observe(target: Arc<Mutex<Target>>, latest: Arc<Mutex<Option<Snapshot>>>) {
    let mut current = Target::default();
    let mut client = None;
    let mut retry = Instant::now();
    loop {
        let requested = target.lock().unwrap().clone();
        if requested != current {
            current = requested;
            client = None;
            retry = Instant::now();
        }
        if current.generation == 0 {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if client.is_none() && Instant::now() >= retry {
            client = couch_control::StreamingConnection::load(&crate::connections::file(
                &current.connection,
                "androidtv",
            ))
            .ok()
            .filter(|c| c.kind() == "androidtv")
            .and_then(|c| Observer::connect(c.address(), Duration::from_secs(2)).ok());
            retry = Instant::now() + Duration::from_secs(10);
        }
        if let Some(c) = client.as_mut() {
            match c.poll(Duration::from_millis(200)) {
                Ok(status) => {
                    if *target.lock().unwrap() == current {
                        *latest.lock().unwrap() = Some(Snapshot {
                            generation: current.generation,
                            status,
                            at: Instant::now(),
                        });
                    }
                }
                Err(_) => {
                    client = None;
                    if *target.lock().unwrap() == current {
                        *latest.lock().unwrap() = None;
                    }
                    retry = Instant::now() + Duration::from_secs(10);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn timeline(position: f64, duration: Option<f64>, live: bool) -> (String, Option<String>, f32) {
    let duration = duration.filter(|d| d.is_finite() && *d > 0. && !live);
    let position = if position.is_finite() {
        position.max(0.)
    } else {
        0.
    };
    let position = duration.map_or(position, |d| position.min(d));
    (
        clock(position),
        duration.map(clock),
        duration.map_or(0., |d| (position / d) as f32),
    )
}
fn clock(seconds: f64) -> String {
    let seconds = seconds.min(359_999.) as u64;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}
#[derive(Default)]
struct ArtSchedule {
    key: Option<ArtKey>,
    next: Option<Instant>,
}
impl ArtSchedule {
    fn due(&mut self, wanted: Option<ArtKey>, now: Instant) -> Option<ArtKey> {
        if wanted != self.key {
            self.key = wanted;
            self.next = Some(now);
        }
        if self.next.is_some_and(|at| now >= at) {
            self.next = None;
            self.key.clone()
        } else {
            None
        }
    }
    fn failed(&mut self, now: Instant) {
        self.next = Some(now + Duration::from_secs(15));
    }
}
fn artwork(target: Arc<Mutex<Option<ArtKey>>>, reply: Arc<Mutex<Option<ArtReply>>>) {
    let mut schedule = ArtSchedule::default();
    loop {
        let wanted = target.lock().unwrap().clone();
        if let Some(key) = schedule.due(wanted, Instant::now()) {
            let pixels = fetch(&key.url);
            if pixels.is_none() {
                // A transient DNS/server failure must not suppress this image forever.
                // Never log artwork URLs: providers may put credentials in the query.
                eprintln!("couch-gui: Android artwork unavailable; retrying in 15 seconds");
                schedule.failed(Instant::now());
            }
            if target.lock().unwrap().as_ref() == Some(&key) {
                *reply.lock().unwrap() = Some(ArtReply { key, pixels });
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn fetch(url: &str) -> Option<Pixels> {
    // URLs come from the paired TV. No device credentials accompany artwork.
    let parsed: ureq::http::Uri = url.parse().ok()?;
    if !matches!(parsed.scheme_str(), Some("http" | "https"))
        || parsed.authority()?.as_str().contains('@')
    {
        return None;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(4)))
        .max_redirects(3)
        .build()
        .into();
    let mut response = agent.get(url).call().ok()?;
    let mut data = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut data)
        .ok()?;
    if data.len() > 4 * 1024 * 1024 {
        return None;
    }
    decode(&data)
}
fn decode(data: &[u8]) -> Option<Pixels> {
    let mut reader = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .ok()?
        .resize_to_fill(480, 800, image::imageops::FilterType::Triangle)
        .to_rgba8();
    Some(Pixels {
        width: decoded.width(),
        height: decoded.height(),
        data: decoded.into_raw(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_handles_live_missing_invalid_and_overrun_values() {
        assert_eq!(
            timeline(75., Some(100.), false),
            ("1:15".into(), Some("1:40".into()), 0.75)
        );
        assert_eq!(timeline(120., Some(100.), false).2, 1.);
        assert_eq!(
            timeline(f64::NAN, Some(f64::INFINITY), false),
            ("0:00".into(), None, 0.)
        );
        assert_eq!(timeline(120., Some(100.), true), ("2:00".into(), None, 0.));
        assert_eq!(clock(3661.), "1:01:01");
    }
    #[test]
    fn failed_art_retries_after_backoff_and_new_sessions_do_not_wait() {
        let now = Instant::now();
        let key = ArtKey {
            generation: 1,
            session: 1,
            url: "https://example.test/art.jpg".into(),
        };
        let mut schedule = ArtSchedule::default();
        assert!(schedule.due(Some(key.clone()), now).is_some());
        schedule.failed(now);
        assert!(schedule
            .due(Some(key.clone()), now + Duration::from_secs(14))
            .is_none());
        assert!(schedule
            .due(Some(key.clone()), now + Duration::from_secs(15))
            .is_some());
        assert!(schedule
            .due(Some(key.clone()), now + Duration::from_secs(30))
            .is_none());
        schedule.failed(now + Duration::from_secs(30));
        let next = ArtKey {
            generation: 2,
            ..key
        };
        assert!(schedule
            .due(Some(next.clone()), now + Duration::from_secs(31))
            .is_some());
        assert!(schedule.due(None, now + Duration::from_secs(32)).is_none());
        assert!(schedule
            .due(Some(next), now + Duration::from_secs(33))
            .is_some());
    }
    #[test]
    fn invalid_artwork_falls_back_without_panicking() {
        assert!(decode(b"not an image").is_none());
    }
}
