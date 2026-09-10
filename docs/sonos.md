# Sonos LAN client

`clients/couch-sonos` is a blocking Rust library and JSON-output CLI for existing
Sonos systems on the local IPv4 network. Call it from a worker thread. It uses
legacy Sonos UPnP/SOAP over HTTP port 1400 without cloud registration or credentials.
It does not provision speakers or implement the official cloud Control API.

## Usage

Build on the host with `cd clients && cargo build --release -p couch-sonos --locked`.
For the remote, run `cargo build --release -p couch-sonos --locked --target
armv7-unknown-linux-musleabihf` from `clients/` so its linker configuration applies.
This branch adds the library and CLI; runtime image packaging, web setup, model
configuration and remote GUI controls follow in a separate integration change.

```sh
couch-sonos discover
couch-sonos 192.168.1.50 status
couch-sonos 192.168.1.50 play
couch-sonos 192.168.1.50 pause
couch-sonos 192.168.1.50 volume 25
couch-sonos 192.168.1.50 mute
couch-sonos 192.168.1.50 unmute
```

`stop`, `next`, and `previous` are also available. Play resumes the existing
source/queue; it does not select music. A successful mutation prints
`{"acknowledged":true}`; query `status` separately to observe current state.
A network timeout has an unknown command outcome and is never retried automatically.

Library entry points are `discover`, `Client::connect(Ipv4Addr)`, `player`,
`coordinator`, `status`, `playback(Playback)`, `volume`, `muted`, `set_volume`, and
`set_muted`. Errors distinguish transport, malformed responses, unsupported
services, HTTP status, UPnP fault codes, invalid volume, and non-coordinator playback.

## Group behavior and compatibility

Playback targets the selected player only after querying current topology and
verifying it is the group coordinator. It affects that coordinator's current
playback group. Selecting a member returns `NotCoordinator` with the coordinator
UUID; the caller must explicitly select the coordinator's address. The client
never follows topology URLs, forwards commands, or changes group membership.
Topology can change between the check and command; there is no atomic group lock.
Volume and mute target the selected player's Master channel, not group volume.
`status.transport` is the selected player's reported state, which can be a proxy
state on non-coordinators.

Compatibility is capability-based: the description must identify a Sonos
ZonePlayer and advertise AVTransport, RenderingControl, and ZoneGroupTopology v1.
The initial implementation has fixture validation only, not a tested speaker-model
matrix. Firmware that removes or restricts legacy UPnP will not be supported.
Source-dependent commands such as next on live radio can return UPnP faults.
No grouping, queue editing, music-service authentication, push events, or artwork
is implemented yet.

## Network and parser boundaries

Discovery sends one SSDP M-SEARCH and listens for three seconds, returning at most
256 responder IPv4 addresses. These are candidates until `connect` validates the
device description; advertised LOCATION URLs are never fetched. Multicast routing
and the host's selected interface determine discovery reachability; explicit IP
addresses work without discovery.

HTTP requests have a five-second total deadline and 512 KiB body limit; a composite
status operation makes four sequential requests. Environment proxies and redirects
are disabled. Control paths must remain on the selected origin. XML is parsed with
DTD/entity expansion disabled, response action namespaces are checked, and SOAP
arguments are escaped. Local HTTP provides no peer authentication or encryption;
this client assumes a trusted LAN, like the legacy protocol itself.

## Validation

Run `cargo test -p couch-sonos --locked` and
`cargo fmt -p couch-sonos --check` from `clients/`. Tests use loopback HTTP fixtures,
not speaker commands. They cover request arguments, escaped XML, entity rejection,
body limits, typed faults, coordinator refusal, discovery filtering, and status.
Physical acceptance still needs discovery, coordinator/member behavior, playback,
and per-player volume/mute checks on a consenting test system.

## Protocol sources

Sonos documents the separate cloud service in [About Control API](https://docs.sonos.com/reference/about-control-api)
and LAN requirements in [Configure your firewall](https://support.sonos.com/en/article/configure-your-firewall-to-work-with-sonos).
The local service wire format and coordinator checks were checked against the
[SoCo implementation](https://github.com/SoCo/SoCo/blob/master/soco/core.py) and its
[service definitions](https://github.com/SoCo/SoCo/blob/master/soco/services.py),
plus the [device-derived AVTransport descriptions](https://github.com/svrooij/sonos-api-docs/blob/main/docs/services/av-transport.md)
and [RenderingControl descriptions](https://github.com/svrooij/sonos-api-docs/blob/main/docs/services/rendering-control.md).
These are independent implementations and device-derived documentation, not a
Sonos commitment to continued legacy protocol support.
