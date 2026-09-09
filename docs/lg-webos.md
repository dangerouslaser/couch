# LG webOS TV

Couch's Rust `couch-webos` client controls LG TVs over the SSAP WebSocket
protocol. The Rust/Wasm configuration UI uses the Rust daemon's authenticated
`/api/webos` routes; credentials are never included in exported house config.

## Setup

Open **Connections → Add a connection → LG webOS TV**. Turn on the TV,
enter its IP address, click **Pair TV**, and approve the prompt on the TV.
If needed, enable LG Connect Apps/mobile-device control in the TV settings.
Pairing waits up to 60 seconds. Saved credentials can be checked and adopted
with **Test connection & save**, without prompting again.

Then open **Rooms & devices**, choose a room and the LG connection, and add
the TV. Connection settings and the room's TV card provide status, volume,
mute, input selection and app launch controls. Input/app buttons change the TV
only when clicked. One LG TV is supported currently; re-pairing with another
TV replaces this connection's private credentials. Removing the connection
requires removing its room assignments first and retains credentials.

Encrypted `wss://IP:3001/` is the default. Explicit pairing trusts and pins
the TV's certificate. Normal connections require that same certificate.
The optional legacy checkbox uses unencrypted `ws://IP:3000/`; there is no
silent security downgrade. Credentials live at `/opt/couch/webos-connection.json`
(mode 0600); `COUCH_WEBOS_CONNECTION` overrides that location for daemon tests.

## Client and validation

```sh
(cd clients && cargo test -p couch-webos)
couch-webos pair wss://192.168.1.176:3001/ /private/path/tv.json
couch-webos /private/path/tv.json status
couch-webos /private/path/tv.json watch
```

The library also supports playback, navigation buttons, subscriptions and
Wake-on-LAN. WOL needs a MAC address and network wake enabled on the TV.
Open the TV device in a room (or an activity with the LG TV as its source)
to enter the native Slint control screen:

| Control | Action |
| --- | --- |
| D-pad / OK | Navigate / select on the TV |
| Back | Back on the TV; stays in TV control mode |
| Volume + / − | TV volume up / down |
| Channel + / − | TV channel up / down (TV/app support required) |
| Menu | TV menu |
| Return to Couch (touch) / Home key | Exit TV control mode |

Touch buttons also provide TV Home, explicit mute/unmute, play/pause and
reconnect. TV navigation has no per-key acknowledgement. Volume status is
read after volume commands and every five seconds while idle. The native
controller owns persistent encrypted control/navigation sockets on a worker;
it does not block rendering. Its input queue is bounded, old queued keys
expire after 750ms, and leaving the screen invalidates queued commands and
late UI replies. An already-sent command cannot be recalled. Failed commands
are never automatically retried. Activity startup steps and WOL are not yet
mapped to this screen; turn on the TV before connecting.

The browser controls reconnect per operation. The library also supports
interleaved push subscriptions.

Validated against the physical TV at 192.168.1.176: encrypted pairing,
status, inputs, apps, subscriptions, and unchanged-volume command; the ARM
client also reads status from the remote. Browser checks cover adopting a
saved pairing and adding the TV to a room. Unit tests cover request/event
ordering, rejection, expired pairing, WOL packets, address validation and
configuration round trips. A physical-remote fixture verifies Slint input
through Rust SSAP/pointer sockets for D-pad, OK, Back, volume, channels and
exit. The physical GUI also raises and restores volume on the actual LG TV.
Navigation-socket establishment is verified on that TV;
physical wake is not yet verified.

Protocol references: [Home Assistant's maintained SSAP client](https://github.com/home-assistant-libs/aiowebostv)
and [webOS integration setup](https://www.home-assistant.io/integrations/webostv/).
