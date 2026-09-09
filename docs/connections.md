# Connections and private settings

Add named connections in the web UI, then assign their devices in **Rooms & devices**.
Multiple Hue bridges, Home Assistant servers, Kodi players and LG TVs are supported.
A device keeps its upstream resource ID and a separate connection ID. The same
Home Assistant entity ID or Hue resource ID can appear on different servers.

Infrared has one connection: the remote's built-in blaster. Each IR device in a
room chooses its own codeset. No external IR transmitter is required. Actual IR
sending/learning remains unavailable on the current kernel.

## Storage and migration

Private files are beside `config.json`, under `connections/<connection-id>/`:

- `hue-connection.json`: URL, application key, pinned certificate.
- `ha-connection.json`: server URL and access token.
- `webos-connection.json` and `webos-wake.json`: TV pairing and wake address.
- `kodi-connection.json`: bound host, HTTP port, username/password, control mode.

Credentials are mode 0600 and absent from exported house configuration. The daemon
copies former singleton files into their original named connection on startup,
keeping the originals. `connection-legacy-map.json` prevents reassignment of a
legacy pairing after its connection is deleted. Retained credential directories
reserve their IDs; creating another connection with the same name gets a new ID.
Deletion is rejected while devices or scenes still reference the connection.

Provider operations use `/api/connections/<id>/<hue|ha|webos|kodi>/...`.
Legacy provider-only routes reject ambiguous requests when several connections
exist. Per-connection operation locks allow independent servers to operate at once.
Hue state and SSE subscriptions are separate per bridge; UI cache keys include the
connection ID. The upstream ID is stripped before sending any command.

## Kodi credentials

Create the Kodi connection with its host and TCP port, then open **Kodi web access**.
Enter HTTP port, username and password, choose control mode, and **Test & save**.
TCP retains push notifications and uses credentials for artwork only. Authenticated
HTTP uses the login for commands and artwork, with playback refreshes every five
seconds and immediately after commands. Kodi's TCP interface does not authenticate.

A blank password field preserves an existing login. **Use an empty password**
explicitly clears it. Failed tests leave saved credentials intact. Changing the
host invalidates the old login. Reopen a running activity after changing its login
or transport. See [Kodi services](https://kodi.wiki/view/Settings/Services/Control).

## Validation

Run `cargo test` in model, clients, daemon and ui. Daemon tests cover credential
privacy, rejected login preservation and legacy migration; model tests cover
repeated server resource IDs and one blaster with multiple codesets.
Browser/device fixture checks additionally cover two bridges, HA servers and TVs,
custom authenticated Kodi playback/artwork, and creating multiple named connections.
