# UniFi Protect camera client

`clients/couch-unifi-protect` is a read-only Rust client for the official local
Protect Integration API. It discovers cameras, retrieves a bounded JPEG snapshot,
and supplies an existing RTSPS URL to a future video player. It does not decode
video, display a live feed in Slint, enable shared camera streams, change camera
settings, receive events, or authenticate with a username/password.

Protocol reference checked 2026-09-10: Ubiquiti's
[Protect 7.3.47 OpenAPI specification](https://developer.ui.com/protect/v7.3.47/openapi.json).
The specification's internal `info.version` is `0.0.0`; the documentation URL
identifies the application version reviewed. Older/newer console compatibility
requires checking the actual application version and these endpoints. Ubiquiti
separates local application APIs from its cloud Site Manager API.
[Official API overview](https://help.ui.com/hc/en-us/articles/30076656117655-Getting-Started-with-the-Official-UniFi-API).

## API and stream behavior

The configured origin is the local console, for example `https://nvr.unifi`.
Requests carry `X-API-Key` and use `/proxy/protect/integration/v1`:

| Operation | GET path below the prefix |
|---|---|
| Version | `/meta/info` |
| Camera discovery | `/cameras` |
| Existing streams | `/cameras/{id}/rtsps-stream` |
| Snapshot | `/cameras/{id}/snapshot?channel=main&highQuality=false` |

Camera results include ID, nullable name, model/type, state and package-camera
support. Stream quality is selected explicitly: low, medium, high or package.
An absent/null quality yields `StreamNotEnabled`, with no automatic fallback.
For initial HA100 testing choose low quality; the API does not establish the
codec, profile, frame rate or decodability of that stream.

The official API also exposes POST/DELETE for per-camera RTSPS qualities. Those
operations configure shared stream availability; the documented API does not
provide a per-viewer lease/token with expiry. This client intentionally uses
existing streams. Enable the desired quality explicitly in Protect before testing.
A future explicit stream-management feature must account for other consumers;
it must not delete a preexisting shared stream when a Couch view closes.

`LiveView` contains the returned RTSPS URL, unchanged (including `?enableSrtp`).
Its nonzero lifetime of at most 300 seconds is a **local descriptor validity window**, not a server
lease or automatic URL revocation. `url()` fails after expiry; consuming `close()`
drops the descriptor. The eventual player owns stopping its media connection on
close, expiry, backgrounding, Wi-Fi loss and application shutdown. This library
opens no video sockets and cannot stop a player to which a caller copied the URL.
RTSPS URLs are sensitive and deliberately omitted from Debug output.
[Official stream schemas and endpoints](https://developer.ui.com/protect/v7.3.47/openapi.json).

## Trust and credentials

Public trust roots and hostname verification are the default. `Client::new`
accepts an explicit private CA PEM to replace roots for that client; the
certificate must still validate for the configured console hostname/IP.

For a console with a self-signed certificate whose SAN does not include a local
alias, `Client::new_pinned` offers an explicit alternative trust model. Supply
exactly 64 hexadecimal characters: SHA-256 of the leaf certificate's DER bytes.
The pin binds **only the configured HTTPS origin (scheme, host and port)** to that
exact certificate. It replaces CA-chain, certificate-date and hostname checks;
it is not an additional check layered on public CA validation. TLS 1.2/1.3
handshake signatures are still verified using the pinned certificate's public
key. The entire handshake completes before an API key or HTTP headers are sent.
A different certificate, even with the same name or signing CA, fails closed.
There is no generic insecure mode or automatic trust-on-first-use fallback.

Verify the fingerprint through a trusted administrative channel before enabling
this mode. Merely observing a certificate on an untrusted connection does not
prove its identity. Certificate renewal requires separately confirming a new pin.
Do not populate a pin automatically from a failed connection. A public certificate
file can be fingerprinted offline with `openssl x509 -in console.pem -outform DER |
openssl dgst -sha256`; this does not itself establish trust. Pins and private-CA
configuration are mutually exclusive in the example. The custom connector uses
ureq's unversioned API, so its exact dependency version is locked and upgrades
must run the TLS peer fixtures.

Redirects and environment HTTP proxies are disabled. Requests have a configurable
nonzero maximum of at most 30 seconds, JSON responses
are limited to 2MiB, snapshots to 8MiB and camera lists to 512. JPEG signature checks
do not replace decoder dimension/pixel limits. Error bodies are not surfaced,
and API keys have redacted Debug output and zeroizing owned storage. Keys are not
sent to RTSPS destinations; media authentication/TLS is the player's responsibility.

By default stream URLs must have the same host as the API origin. If Protect
returns a LAN IP while the API uses `nvr.unifi`, explicitly authorize that trusted
host with `with_stream_host`. This permits returning its descriptor only: it does
not rewrite the URL, contact the host, forward the API key or weaken media TLS.
Keep the `enableSrtp` query and do not silently downgrade RTSPS to RTSP.

## Operator-run check

A private template was prepared outside Git at
`~/.local/state/couch/unifi-protect-test.json` on the development Mac. Fill it
locally; never paste the API key into a command or chat. Its shape is:

```json
{
  "origin": "https://nvr.unifi",
  "api_key": "",
  "private_ca_pem": null,
  "certificate_sha256": null,
  "stream_host": null,
  "camera_id": null
}
```

Keep mode0600. Set `private_ca_pem` to an absolute trusted certificate path if
needed, or set `certificate_sha256` only after explicitly verifying the exact
console fingerprint. Leave both null to use ordinary public trust. Set `stream_host` only if the returned stream host differs, and choose
`camera_id` only when ready to validate that camera's existing low-quality URL.
The key must be authorized by this console's Integration API; cloud-account key
availability does not itself prove local Protect access. Use the console's own
version-specific integration instructions and minimal available camera/read
permissions. The client reports authentication, permission, unavailable endpoint,
rate limiting and offline errors separately; it has no legacy login fallback.

Once the NVR update has finished and the operator explicitly chooses to connect:

```sh
cargo run --manifest-path clients/Cargo.toml -p couch-unifi-protect \
  --example check -- "$HOME/.local/state/couch/unifi-protect-test.json"
```

The check prints only application version, visible camera count and, if selected,
whether a low-quality descriptor validated. It does not print stream URLs or open
video. No NVR call was made while implementing this client.

## Validation and remaining live-view work

Run `cargo test --locked -p couch-unifi-protect` and
`cargo clippy --locked -p couch-unifi-protect --all-targets -- -D warnings` in
`clients/`. Local TLS peers exercise trusted/private CA behavior, rejected
untrusted chains and wrong hostnames, matching and incorrect certificate pins,
self-signed hostname aliases, forged handshake signatures, API-key headers, exact routes, redirects,
status handling, response limits, camera schemas, JPEG validation, stream quality,
explicit stream hosts and local descriptor expiry. Fixtures use generated keys
and loopback listeners; no real camera or console is contacted.

Physical acceptance still needs the updated NVR's application version, a locally
entered API key and valid TLS trust. Confirm camera visibility, low/package stream
availability and actual response schemas. Then separately implement and test an
RTSPS/SRTP-capable player with verified media TLS, bounded packet/frame queues,
codec negotiation, timeout/cancellation and a Slint rendering path. Measure
480x800 display scaling, frame latency, CPU/RAM, audio policy and battery cost on
the Cortex-A7 HA100. Hardware decode availability is unproven; do not claim that
returning a stream URL demonstrates a playable live feed on the remote.
