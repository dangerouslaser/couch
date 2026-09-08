# Philips Hue lights

`clients/couch-hue` is a Rust client for direct local Hue API v2 control. It does
not require Home Assistant, a Hue cloud account, or a kernel change. Start with
on/off and brightness; color, scenes, grouped lights and event streaming are not
implemented yet. One bridge connection is supported per remote.

## Connect and add lights

1. Open the remote's web editor, pair with its on-screen PIN, and open **Connections → Philips Hue**.
2. Enter the bridge IP address (for example `192.168.1.157`). Press the bridge's
   round link button, then **Pair bridge**. A failed pairing leaves saved settings intact.
3. Create a room under **Rooms & devices** if needed. Click **Find lights**, choose
   a destination room, and **Add light**. Discovery never adds devices automatically.
4. Open that room on the remote. Touch or use the D-pad to select a light, turn
   it on/off, adjust brightness, or refresh its state. The web editor also has
   controls for testing before import. Zero-percent brightness means off.

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
House devices store only `{"via":"hue","light_id":"<v2 light UUID>"}`.
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
“link button not pressed”; real pairing and household-light validation are pending.
