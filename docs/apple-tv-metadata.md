# Apple TV now-playing client

`couch_appletv::metadata` implements a separate, native Rust AirPlay 2 / MRP
connection for modern tvOS. It is experimental: loopback protocol peers and wire
fixtures pass, but no physical Apple TV has validated this implementation yet.
The existing Companion control and pairing API remains independent.

## Pairing and observation

Discover the TV's `_airplay._tcp.local.` service and use its advertised port with
`metadata::Settings`; do not use the Companion port or assume a fixed port.
`metadata::Pairing::begin` explicitly requests an on-screen PIN. `finish` verifies
SRP proof and the signed accessory identity, returning separately typed AirPlay
credentials. Store them privately; they are redacted in Debug output. Companion
credentials are not accepted by this API.

`metadata::Client::connect` verifies that saved identity, opens encrypted control,
event and data channels, exchanges MRP device information and subscribes to
now-playing updates. The AirPlay session is remote-control-only: it does not load
media, launch apps or request playback. Call `poll` on a dedicated worker at least
once per second. It answers events and sends AirPlay feedback every two seconds.
Ordinary polls wait about 101 ms; feedback requests have a five-second deadline.
No operation or pairing is automatically retried. Any poll error closes the
session and clears metadata; reconnect explicitly with the saved credentials.

`NowPlaying` exposes app/player/item identity, title, subtitle, artist, album,
series/season/episode, genre, playback state, duration, elapsed position, rate,
live status and artwork availability/identifier/HTTP(S) URL/MIME type. Missing
values remain absent. App/player changes select the matching cached state;
content-item updates merge optional fields. Queue replacement removes stale
metadata. Apple epoch timestamps are converted to Unix seconds; `position_at`
advances only playing state and clamps known duration. Large clock discontinuities
leave the last sample unchanged.

Artwork URLs are metadata only. The client does not fetch URLs, decode embedded
artwork or request artwork-only queue assets. Applications differ in the fields
they expose; an absent title or artwork URL is not necessarily a transport error.
The Couch configuration UI still pairs Companion only, and the device screen
still uses Companion controls. AirPlay pairing storage and metadata rendering
are separate UI follow-up work; this change adds the client API and probe.

## Physical test probe

Run from the `clients` workspace, substituting the TV's actual AirPlay port.
Choose a credential path outside the repository in a private directory:

```sh
cargo run -p couch-appletv --example now_playing -- pair 192.168.1.20 7000 /private/path/apple-tv-airplay.json
cargo run -p couch-appletv --example now_playing -- watch 192.168.1.20 7000 /private/path/apple-tv-airplay.json
```

`pair` prompts for the displayed PIN and refuses to overwrite an existing file.
Files are mode 0600 on Unix; Windows users should use their private user directory
with its access controls. `watch` prints changed metadata as JSON until Ctrl-C.
It does not send playback commands. Test active music/video apps, pause/resume,
seek, app switching, sleep/wake, disconnect and saved-credential reconnection.
Record TV model, tvOS version and app names, without recording PINs or credentials.

## Validation and protocol references

Run `cargo test -p couch-appletv` and
`cargo clippy -p couch-appletv --all-targets -- -D warnings` in `clients`.
Tests include a signed pair-verify peer with fragmented authenticated HAP records,
three-channel AirPlay setup, subscription and pushed metadata, event replies and
connection loss. Other fixtures cover partial updates, active-app isolation,
empty queues, numeric/URL sanitation, cache limits, malformed framing, bounded
plist expansion, ciphertext tampering and redacted credentials. Existing
Companion pairing/control tests exercise the shared authentication primitives.

`src/metadata/fixtures/now-playing.bplist` is synthetic, generated independently
with Python's `struct` and `plistlib`, using MRP field numbers. It contains no
captured device data, credentials or third-party media. The fixture encodes a
SetState (type 4, extension 9) and SetNowPlayingClient (type 46, extension 50),
with a default player, title/artist, 240-second duration and 12.5-second position.

Wire details were checked against pyatv commit
`b277a4c8222ecdcbaab8a24e3e713ca44765adb4`:

- [AirPlay session setup](https://github.com/postlund/pyatv/blob/b277a4c8222ecdcbaab8a24e3e713ca44765adb4/pyatv/protocols/airplay/ap2_session.py)
- [AirPlay data/event framing](https://github.com/postlund/pyatv/blob/b277a4c8222ecdcbaab8a24e3e713ca44765adb4/pyatv/protocols/airplay/channels.py)
- [AirPlay HAP pairing](https://github.com/postlund/pyatv/blob/b277a4c8222ecdcbaab8a24e3e713ca44765adb4/pyatv/protocols/airplay/auth/hap.py)
- [MediaRemote schemas](https://github.com/postlund/pyatv/tree/b277a4c8222ecdcbaab8a24e3e713ca44765adb4/pyatv/protocols/mrp/protobuf)
- [Player state handling](https://github.com/postlund/pyatv/blob/b277a4c8222ecdcbaab8a24e3e713ca44765adb4/pyatv/protocols/mrp/player_state.py)

This is a limited protobuf projection rather than a full MRP implementation.
Unknown messages/fields are ignored; transport sizes, pending messages, plist
expansion and cached player data are bounded. Protocol compatibility, sustained
feedback and application-specific metadata require the physical tests above.
