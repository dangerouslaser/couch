# Kodi contextual OK

Couch's default Kodi OK and activity mappings to Kodi `ok` send
`Input.ButtonEvent` with `{"button":"select","keymap":"R1","holdtime":0}`.
Kodi resolves the physical remote key against its current window and user
keymaps. Its defaults select items in browsers, open the OSD in fullscreen
video/music, and select focused controls when an OSD or dialog is open.
Background playback does not turn browsing OK presses into OSD requests.

Previously Couch sent `Input.Select`, which directly dispatches the select
**action** and bypasses the contextual keymap. Checking Couch's cached playback
state would also be incorrect: media can play while Kodi displays its library.
The new path sends one request without extra state polling. Rejections are
reported without retrying a different action. Custom activity overrides to
other functions remain unchanged; existing `ok` mappings need no migration.

Sources:
- [Kodi input implementation](https://github.com/xbmc/xbmc/blob/Omega/xbmc/interfaces/json-rpc/InputOperations.cpp)
- [Kodi default remote keymap](https://github.com/xbmc/xbmc/blob/Omega/system/keymaps/remote.xml)
- [Kodi key translation](https://github.com/xbmc/xbmc/blob/Omega/xbmc/input/keymaps/ButtonTranslator.cpp)

The connected CoreELEC instance advertised `Input.ButtonEvent` in JSON-RPC
introspection (API 13.201.0). Rust regression coverage checks the exact remote
key event and error propagation. HA100 fixtures verify default OK during idle
and playback, and mapped OK through the shared daemon service.
