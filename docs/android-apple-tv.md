# Android TV and Apple TV clients

The new `couch-androidtv` and `couch-appletv` crates are native Rust libraries in
`clients/`. They do not invoke Python, ADB or an external remote-control program.
Both are **experimental**. Connections now offers network discovery and explicit
PIN pairing. The GUI and daemon are deployed on the development remote; initial
Android TV pairing and authenticated status reads have passed on a Xiaomi TV.
Apple TV physical validation remains pending. Protocol-peer tests and ARMv7
compilation are not physical TV validation.

## Android TV / Google TV

`Identity::generate()` creates a separate RSA-2048 client identity. Keep this
identity and the returned `Credentials` in the connection's private credential
store, never in exported house configuration or Git. `Pairing::begin()` opens
TLS port 6467 and requests the six-character hexadecimal code displayed by the
TV; `finish(code)` binds that code to both endpoints' RSA keys. Successful pairing
returns the TV certificate pin plus the reusable client identity.

`Remote::connect()` opens a persistent mutually authenticated TLS connection to
port 6466 and requires the saved TV certificate. Poll the connection at least
once per second to answer keepalives and consume power/volume updates. Navigation,
volume, playback, channel and power key codes use `press(Button::...)`; `launch()`
sends an app link. A successful write means the command was sent, not that the TV
performed it. No command is automatically retried. These calls belong on the
shared control service's worker, not the render thread.

The target needs Android TV Remote Service, ordinarily present on Android/Google
TV. It does not require enabling ADB. Feature negotiation excludes voice and IME
for this first version. Text input, current-app tracking and voice streaming remain
future additions. [Protocol implementation and schemas](https://github.com/tronikos/androidtvremote2)

Needed for hardware testing: **TV address/model and its displayed pairing code**.
Default ports are configurable. The Connections finder uses Bonjour and also permits manual address entry.

## Apple TV

Use the Bonjour-advertised `_companion-link._tcp.local.` port with `Settings`;
there is no universal fixed Companion port. `Pairing::begin()` requests the TV's
four-digit PIN and retains that connection while the user enters it. `finish()`
performs SRP-3072/SHA-512 authentication and verifies the server proof and signed
accessory identity before returning credentials.

`Remote::connect()` verifies the saved device identity with X25519/Ed25519,
derives directional keys, and opens a ChaCha20-Poly1305 authenticated Companion
session. `press()`, `playback()`, `launch()` and `apps()` provide navigation,
media controls and app management; `close()` ends the remote session. Frames,
OPACK depth/object counts, queued events, and operation waits are bounded.
Credentials are redacted in Debug output and must be stored privately per TV.

Modern tvOS playback metadata is **not** supplied by a plain standalone MRP
socket: tvOS 15 moved that path into AirPlay 2. This implementation starts with
Companion controls and does not claim artwork, timeline, media streaming or
full now-playing support. Those need a separate AirPlay/MRP implementation.
[pyatv protocol documentation](https://pyatv.dev/documentation/protocols/),
[Companion implementation](https://github.com/postlund/pyatv/tree/master/pyatv/protocols/companion)

Needed for hardware testing: **Apple TV address, tvOS version and on-screen PIN**.
Discovery fills the Companion port. No Apple ID password is requested; physical
TV validation remains pending.

## Validation

Run `cargo test -p couch-androidtv -p couch-appletv` from `clients/`.
Tests include a real mutual-TLS Android peer through pairing, negotiation,
keepalive and key/app commands; an encrypted Apple Companion peer through
pair-verify/session setup/navigation/playback/app commands; malformed framing,
OPACK/TLV limits, authenticated-encryption rejection and certificate-bound codes.
Apple SRP output is compared with an independent `srptools==1.0.1` vector, and
accessory signatures/server proofs must validate before pairing completes.

The libraries compile for `armv7-unknown-linux-musleabihf`. The host browser bundle
and configuration daemon also build. Model, broker and daemon regression tests
cover provider resolution, private credentials and pairing session handling. A
Playwright fixture verified both connection types through mocked discovery, PIN
entry, saved status and mobile layout without contacting a TV. This is client-side
validation; every kernel build remains on Ollie.

## Couch integration

Add a named **Android / Google TV** or **Apple TV** connection, find/select the
TV (or enter its address), request its code, then **Verify & save pairing**.
Pairing sessions expire after two minutes and can be cancelled; private JSON
credentials are written atomically with mode 0600 under
`connections/<id>/{androidtv,appletv}-connection.json`. Ordinary connection reads
return only paired status, address and port. Exported configuration contains only
the named provider reference.

Add the TV to a room. The room editor provides basic test controls; activities
can map its supported functions to physical buttons, ordered start/stop steps,
and custom command pages. A dedicated full-screen Android/Apple TV media view is
not implemented. Use custom activity pages for the first iteration. Unsupported
Apple functions such as mute and stop are omitted from the capability catalog.

The shared control broker owns each endpoint, answers Android keepalives while
idle, expires stale queued commands, and reconnects only on a later request after
failure. Activity navigation releases retained connection handles. Credentials
changed through pairing replace the next mapped command's connection settings.
Discovery is an explicit three-second IPv4 Bonjour search; IPv6 addresses can be
entered manually, but scoped/link-local IPv6 discovery is not implemented.


## Initial Android TV hardware check

The HA100 paired with a Xiaomi `MiTV-AFMU0` using its six-character on-screen
code. Credentials were saved on the remote, and the authenticated Remote v2
connection reported model/vendor, power and volume state. Home, Right and Left
command requests completed successfully, and the operator confirmed visible
navigation on the TV. A later connection using saved credentials also passed.
Long-running keepalive and network-loss recovery remain separate checks; a
successful command write alone does not prove that the TV performed it. No Apple TV has
been paired during this validation.
