# Philips Hue lights

`clients/couch-hue` is a Rust client for direct local Hue API v2 control. It does
not require Home Assistant, a Hue cloud account, or a kernel change. Start with
on/off and brightness; color, scenes and grouped lights are not
implemented yet. One bridge connection is supported per remote.

## Connect and add lights

1. Open the remote's web editor, pair with its on-screen PIN, and open **Connections → Philips Hue**.
2. Enter the bridge IP address (for example `192.168.1.157`). Press the bridge's
   round link button, then **Pair bridge**. A failed pairing leaves saved settings intact.
3. Open **Rooms & devices**, create or open a room, then choose the saved Hue
   connection under **Add devices to this room**. Search the discovered lights and
   click **Add to this room**. Discovery never assigns devices automatically.
4. Open that room on the remote. Devices appear in one flat list. Highlight a light
   and press OK to toggle it. Tapping selects a row, matching room navigation.
   Physical Back returns home. Use
   **Show light controls** on an assigned device in the web editor for brightness.
   Zero-percent brightness means off.

Unreachable lights display as unavailable. Commands are acknowledged by the
bridge; refresh to see reported state. This is not proof of physical illumination.

## Connection and credentials

Requests use HTTPS with a five-second per-request deadline, no redirects or
proxies, and a 4 MiB response cap. Pairing trusts the selected LAN bridge on first
use and saves its exact certificate. Later requests require the same certificate
and a valid TLS handshake signature. This is certificate pinning, not public-CA
or hostname validation. Pair only on your trusted LAN; deliberate re-pairing is
required after the bridge changes its certificate. Pairing has no certificate
bypass setting for normal control requests.

The application key and certificate are saved atomically with mode `0600` in
`/opt/couch/hue-connection.json`, separate from the exportable house configuration.
The API never returns the key. `COUCH_HUE_CONNECTION` overrides the daemon's path.
New house devices store `{"via":"connection","connection_id":"philips-hue",
"resource_id":"<v2 light UUID>"}`. Existing inline Hue definitions remain readable.
Adding this integration requires updated readers of the shared configuration.

## CLI and validation

```sh
(cd clients && cargo test -p couch-hue)
couch-hue pair 192.168.1.157
couch-hue lights
couch-hue on LIGHT_UUID
couch-hue brightness LIGHT_UUID 40
```

Use `--settings PATH` before a command to override the private settings file.
`node web/tests/hue.mjs` uses an isolated HTTPS fixture and disposable host daemon;
first build with `tools/build-webui.sh --host`. It tests pairing, private storage,
certificate mismatch rejection, discovery, controls, unreachable lights, Hue
error envelopes, room import and mobile layout. No household lights are touched.

Protocol references: [Hue getting started](https://developers.meethue.com/develop/get-started-2/),
[API v2](https://developers.meethue.com/new-hue-api/), and
[HTTPS guidance](https://developers.meethue.com/develop/application-design-guidance/using-https/).
Hue advises against continuous rapid updates through its REST API; Couch sends
explicit user commands and refreshes on demand.

## Device validation (2026-09-08)

Deployed ARMv7 CLI, daemon/browser bundle and Slint GUI. All affected workspace
unit tests passed, as did the isolated Hue browser test and existing Home Assistant
browser regression. On the physical remote, D-pad discovery, on and 40% brightness
passed against an isolated HTTPS bridge fixture; production configuration was
restored afterward. The real BSB002 at `192.168.1.157` responds over HTTPS with
48 discovered lights (42 reachable). Pairing and read-only discovery succeeded;
real household-light command testing remains pending.

## Remote state cache and response time

Room names and devices render from local configuration immediately. Device rows
match the home room list’s cards, icons, typography and moving focus ring. OK
toggles the focused light; physical Back returns home with a 180 ms slide.

The GUI starts a credential-scoped Hue session in the background. A pinned HTTPS
connection subscribes to `/eventstream/clip/v2`. Add/update/delete events trigger
background state snapshots, preserving connectivity information as well as light
state. This version refreshes snapshots on events rather than merging partial
resource payloads. Commands use a separate, reusable HTTPS connection.

With a valid cached state, OK sends only a PUT. The row adopts the target after
bridge acknowledgement; this does not prove the bulb has finished fading. Missing
or expired state falls back to a fresh read; known-unavailable lights are refused.
Failures invalidate the cache. Snapshot generations prevent an older response from
overwriting a command, and overlapping updates schedule another refresh.

Streaming connections reconcile every 60 seconds. On stream failure, cache validity
is revoked, snapshots poll every five seconds, and stream reconnects back off from
one to 30 seconds. An idle stream has a 45-second read timeout so dead connections
recover. Reconnect, credential replacement, and wake from full standby refresh state.
The GUI checks the local cache every 500 ms in Hue-only rooms (five seconds in mixed
rooms); these checks do not normally query the bridge. A short race remains if
another controller changes a light before its push-triggered snapshot completes.

Validation: 32 GUI tests and eight Hue tests pass, including SSE framing/limits,
cache expiry, disconnected state, and stale snapshot/command ordering. On the
physical remote, an isolated HTTPS/SSE fixture verified one-PUT cached toggles,
external changes via push, stream failure with polling, and reconnect recovery.
Fixture cached acknowledgements took 34–55 ms while streaming; these are not
real-bridge/bulb latency measurements. Production credentials and configuration
were preserved. No household lights were changed by automated fixture tests.

[Hue v2 push support](https://developers.meethue.com/new-hue-api/).
