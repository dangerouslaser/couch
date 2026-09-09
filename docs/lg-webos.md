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
These functions are not yet mapped to a dedicated Slint TV control screen or
activity steps. The current browser controls reconnect for each operation;
the library supports persistent connections and interleaved push updates.
Commands are never automatically retried.

Validated against the physical TV at 192.168.1.176: encrypted pairing,
status, inputs, apps, subscriptions, and unchanged-volume command; the ARM
client also reads status from the remote. Browser checks cover adopting a
saved pairing and adding the TV to a room. Unit tests cover request/event
ordering, rejection, expired pairing, WOL packets, address validation and
configuration round trips. Physical wake and navigation are not yet verified.

Protocol references: [Home Assistant's maintained SSAP client](https://github.com/home-assistant-libs/aiowebostv)
and [webOS integration setup](https://www.home-assistant.io/integrations/webostv/).
