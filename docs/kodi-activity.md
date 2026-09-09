# Kodi Cinema activity screen

The native Rust/Slint activity screen implements the selected Cinema design:
full-screen fanart and clearlogo, playback progress, play/pause, seek, chapters,
audio/subtitles, and a Kodi navigation panel. Back closes a sheet or returns
to Couch without stopping playback.

Add a Kodi connection, assign its player to a room, and create an activity
with that device as the source. Attach the activity to a room or area. A
room's Kodi device row also opens the player directly. Startup command steps
are not executed yet; this screen controls playback already running in Kodi.
Enable Kodi's TCP remote-control service (default port 9090).

Physical OK activates the highlighted control; channel keys skip chapters,
volume keys adjust Kodi volume, and Back returns. The timeline supports
touch seeking and directional-key previews committed with OK. Missing chapter
support is handled without failing the player.

Artwork uses Kodi's HTTP service. Optional private `/opt/couch/kodi-web.json`
settings map a Kodi host to its HTTP port and credentials:

```json
{"kodi.local":{"port":8080,"username":"couch","password":"your-private-password"}}
```

Keep this file outside Git and exports. Text and the dark background remain
usable when artwork is unavailable. Artwork decoding and networking run on
workers with bounded queues, download/decode limits and a small decoded-image
cache. Media identity and navigation generations discard stale replies.

Validation: Kodi protocol unit tests, GUI tests, ARM release build, and a
physical-device fixture covering playback, artwork, pause, chapter selection,
volume, Back, and launch from room and area activities. Production Kodi
playback still needs testing against the user's actual Kodi server.
