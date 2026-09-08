# Home Assistant lights

`clients/couch-ha` is a Rust library and CLI for Home Assistant light discovery,
state, on/off and brightness. It uses bounded blocking HTTP requests in a worker,
with connection reuse and verified HTTPS through ureq/rustls. It neither records
voice nor depends on the existing `couch-voice` Assist client.

## Configure and test

Open the remote's configuration editor at `http://192.168.1.127:8090`, pair with
the on-screen PIN, and open **Connections → Home Assistant**. Enter the server
URL and a long-lived access token from your Home Assistant profile. **Test &
save connection** reads the light inventory before saving. A failed test leaves
the previous connection intact. A blank token keeps the saved token only when
the server URL is unchanged.

**Find lights** lists light entities, including unavailable lights. Choose a
room and **Add light** to create a `Light` device with its Home Assistant entity
ID. Create the room under **Rooms & devices** first if needed. Discovery alone
does not add devices or send commands. The light's explicit **Turn on**, **Turn
off**, and **Apply brightness** buttons operate real lights. **Refresh state**
reads confirmed state; service acceptance is not proof of physical delivery.

Credentials live in `/opt/couch/ha-connection.json`, mode 0600, separately from
exportable `/opt/couch/config.json`. API responses never return the token.
`COUCH_HA_CONNECTION` overrides the credential path for isolated host tests.
The CLI alternatively reads `/opt/couch/ha-token` or `--token-file`:

```sh
couch-ha --url http://homeassistant.local:8123 lights
couch-ha --url http://homeassistant.local:8123 state light.study
couch-ha --url http://homeassistant.local:8123 brightness light.study 35
```

Only `light.*` targets are accepted. Unknown/unavailable states are not treated
as off. Brightness follows advertised color modes, with the legacy feature bit
as a fallback; zero explicitly turns the light off. Commands use
`/api/services/light/turn_on` and `turn_off`, never a write to the state cache.
Redirects and environment proxies are disabled. Requests have a five-second
limit and responses a 4 MiB cap; tokens and upstream bodies are excluded from
error messages. Call this client outside the Slint rendering thread.

## Validation and current scope

Nine client tests cover discovery, service payloads, zero brightness, unavailable
and non-dimmable lights, authentication, malformed responses, invalid targets,
timeouts, and private atomic settings. Run `(cd clients && cargo test -p couch-ha)`.
`node web/tests/home-assistant.mjs` runs a loopback-only fake HA server and a
throwaway daemon, checking the real browser setup, rejected credential replacement,
light controls, room import, and mobile layout. The existing 17 daemon tests pass.

`tools/build-webui.sh` builds the WASM bundle and ARM daemon; macOS uses Zig for
ring's cryptographic C primitives, with rust-lld for the final static link.
For the standalone CLI, use the same compiler environment:

```sh
export ZIG="$PWD/build/toolchains/zig-aarch64-macos-0.15.2/zig"
export CC_armv7_unknown_linux_musleabihf="$PWD/tools/arm-musl-cc.py"
(cd clients && cargo build -p couch-ha --release --target armv7-unknown-linux-musleabihf)
```

The ARM CLI and editor are deployed. A real Home Assistant connection and a
user-designated test light are still needed for physical validation. Slint room
controls are the next integration step; this web deployment alone does not make
the existing example home consume the saved configuration. Color, temperature,
groups, other entity domains and push subscriptions are deferred until light
power and brightness have passed live testing.

Protocol references: [Home Assistant REST API](https://developers.home-assistant.io/docs/api/rest/),
[light actions](https://www.home-assistant.io/integrations/light/), and
[ureq configuration](https://docs.rs/ureq/latest/ureq/).
