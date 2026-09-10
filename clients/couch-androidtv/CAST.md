# Read-only Cast metadata

`cast::Observer` supplies optional now-playing information alongside Remote v2.
Run it on a separate worker; its TLS connection must not block physical-control
traffic. Connect to the paired TV's IP on port 8009, poll regularly, and discard
or recreate the observer after an error with caller-controlled backoff.

Only `CONNECT`, `GET_STATUS`, `PING` and `PONG` are emitted. The observer attaches
to an existing receiver app advertising the standard Cast media namespace. It
never launches an app, loads content, seeks or changes playback. App support is
conditional: any app exposing standard Cast metadata can work, but ordinary
Android playback is not guaranteed to publish it. There are no app-specific
Plex, Jellyfin, YouTube or Netflix protocol shortcuts.

`Status` and `NowPlaying` are serializable. `status()` projects position from the
last actual media report only while PLAYING, clamps to a reported duration, and
freezes PAUSED/BUFFERING positions. Polling alone does not refresh that anchor.
Gate the timeline on `position_known`: an omitted initial currentTime is not a
known zero position. Metadata expires after ten seconds without a media report;
app/session changes, idle/empty status and connection errors clear it. Old
transport/session responses are ignored. Artwork accepts absolute HTTP(S) URLs
without embedded username/password; fetching and caching it belongs to the UI.

Cast has a separate self-signed TLS certificate. This observer verifies handshake
signatures but does not authenticate Cast device identity against Remote v2's
certificate. It sends no saved Remote v2 credentials over the Cast connection.
Frames are bounded to 256 KiB, socket operations have timeouts, and a poll handles
at most 64 frames. No failed command or media operation is replayed.

## Verification

Thirteen client tests include fragmented real TLS traffic, an outbound read-only
message whitelist, malformed/oversized frames, partial and stale sessions,
position freshness, missing positions, and invalid artwork URLs.

On 2026-09-09, read-only observations on the user's Android TV reported SmartTube
with title, duration, position and play/pause state. Its metadata had no images.
The Jellyfin client identified itself as Wholphin and reported the title *1917*,
duration, paused position and standard artwork. This does not establish physical
compatibility for every app/device. A subsequent Plex observation supplied
artwork, duration and a paused position, but explicitly empty standard `title`
and `subtitle` fields. Its metadata had no other standard title field, so the UI
should show the app name instead of inventing a content title. Netflix was not
tested.

```sh
cargo run -p couch-androidtv --example cast_status -- 192.0.2.10
```

The example observes for eight seconds and avoids logging artwork URLs, which
may contain transient tokens. It does not start playback.

## Protocol sources

- [Chromium Cast channel protobuf](https://chromium.googlesource.com/chromium/src/+/d61f0b96fc6068df740118c1fb185a027ddc1c96/extensions/common/api/cast_channel/cast_channel.proto)
- [Google Cast MediaStatus](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.MediaStatus)
- [Google Cast MediaInformation](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.MediaInformation)
- [PyChromecast media controller](https://github.com/home-assistant-libs/pychromecast/blob/master/pychromecast/controllers/media.py)
- [PyChromecast socket client](https://github.com/home-assistant-libs/pychromecast/blob/master/pychromecast/socket_client.py)
