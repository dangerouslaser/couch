//! Observe an already-running Cast app. No launch or playback commands exist here.
use couch_androidtv::cast::Observer;
use std::time::{Duration, Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::args()
        .nth(1)
        .ok_or("usage: cast_status <TV IP>")?
        .parse()?;
    let mut observer = Observer::connect(address, Duration::from_secs(2))?;
    let end = Instant::now() + Duration::from_secs(8);
    let mut last = String::new();
    while Instant::now() < end {
        let status = observer.poll(Duration::from_millis(200))?;
        // Avoid logging artwork URLs, which can contain transient access tokens.
        let summary = format!(
            "connected={} app={:?} media={:?}",
            status.connected,
            status.app_name,
            status.now_playing.as_ref().map(|m| (
                &m.title,
                &m.player_state,
                m.duration,
                m.position,
                m.live,
                m.artwork_url.is_some()
            ))
        );
        if summary != last {
            println!("{summary}");
            last = summary;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}
