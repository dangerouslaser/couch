# Kodi activity: research and UI studies

Research date: 2026-09-08. These are interactive HTML/CSS design studies for a
future Rust/Slint screen. Playback, chapters, streams and room names are fixtures;
they do not control Kodi or change the remote. The physical display is 480 × 800.

## Review the designs

Open `index.html` directly, or serve this directory:

```sh
python3 -m http.server 8091 --directory docs/mockups/kodi-activity
```

- **A / Cinema — recommended:** full-screen fanart, clearlogo above a dark
  playback deck, prominent play/pause and direct access to chapters/audio/subtitles.
- **B / Focus:** artwork occupies the upper section; controls have a solid,
  predictable background. Best readability across very bright or busy artwork.
- **C / Quiet:** more artwork and fewer controls. Secondary actions require
  opening a sheet; attractive, but an extra press for frequent chapter use.

The comparison includes playing, paused, absent artwork, idle, disconnected,
live/unseekable and unavailable chapter-list states. Change the accent, scrub,
choose a chapter, change tracks, or open TV controls. Arrow keys move focus;
Enter selects, Space toggles playback, Escape closes the sheet or returns to the
simulated room. The TV-controls sheet maps arrows/Enter to simulated Kodi input.
The exit arrow never implicitly stops playback.

`a.png`, `b.png`, `c.png` and `chapters.png` are native-resolution exports.
The browser prototype is not a device performance benchmark.

## What we already have

`clients/couch-kodi` implements persistent TCP JSON-RPC, HTTP requests,
notification delivery, active-player discovery, now-playing metadata, artwork URL
encoding, play/pause, seek percentage, volume and Kodi menu navigation. Artwork
lookup already supports `tvshow.*` inheritance. `model::Activity` has an owning
room, source device, kind and startup actions. We can extend these foundations.

Missing pieces include an activity runtime, a playback controller attached to
Slint, decoded-image caching, capability discovery, typed chapter/stream APIs,
source selection and activity-specific hardware key routing. The existing
`now_playing()` picks the first active player; a video activity should explicitly
choose the video player and handle no active player.

## Kodi findings that affect the design

| Screen feature | Kodi path / behavior |
| --- | --- |
| Current media | `Player.GetActivePlayers`, then `Player.GetItem` |
| Position and capabilities | `Player.GetProperties`: time, total time, speed, `canseek`, `live`, streams |
| Play/pause | `Player.PlayPause`; reconcile returned speed |
| Scrubbing and chapter timestamps | `Player.Seek` with time/percentage; relative seeks accept seconds |
| Audio and captions | `Player.SetAudioStream`, `Player.SetSubtitle` |
| Kodi menu navigation | `Input.Up/Down/Left/Right/Select/Back` |
| Next item | `Player.GoTo` changes playlist items; it is **not** chapter selection |

These methods are present in the [Kodi 21.2 method schema](https://github.com/xbmc/xbmc/blob/21.2-Omega/xbmc/interfaces/json-rpc/schema/methods.json).
Capability properties are enumerated in its [type schema](https://github.com/xbmc/xbmc/blob/21.2-Omega/xbmc/interfaces/json-rpc/schema/types.json).
Do not hard-code video player ID 1 or assume every source supports seeking.

**Chapters need capability detection.** `Player.GetChapters` is absent from the
21.2 schema and present in the [development method schema](https://github.com/xbmc/xbmc/blob/master/xbmc/interfaces/json-rpc/schema/methods.json).
The [development chapter type](https://github.com/xbmc/xbmc/blob/master/xbmc/interfaces/json-rpc/schema/types.json)
provides a one-based index, optional name and start time in seconds. Discover
support using `JSONRPC.Introspect`; when available, list chapters and seek to the
selected start time. Use “Chapter N” when a name is absent. Older installations
should retain seeking and clearly explain an unavailable chapter list. No
chapter titles, boundaries or thumbnails should be invented in production.
The six named chapters in this prototype are explicitly illustrative. The user's
installed Kodi version and supported methods have not been verified for this study.

**Artwork is optional, not a prerequisite for control.** Request `art`, resolve
`fanart`/`clearlogo` with parent fallbacks, and retrieve images through Kodi's
HTTP `/image/` route. Encode the entire returned `image://` string as one URI
component; do not fetch a decoded NAS path directly. Preserve transparent logos
with contain fitting; crop backdrops separately. Fall back to a text title and a
quiet gradient. Kodi documents the paths and parent prefixes in
[Artwork access](https://kodi.wiki/view/Artwork/Accessing_with_skins_and_JSON-RPC).

**Validate both services at connection setup.** Controls can use TCP while
artwork still needs Kodi's HTTP service and its credentials. Our current
`with_auth()` only applies to an HTTP control handle, so a separate artwork
fetcher must receive web credentials even when commands use TCP. Existing source
comments claiming TCP is enabled by default should not be treated as a guarantee;
check the actual [Kodi remote-control settings](https://kodi.wiki/view/Settings/Services/Control).

## Proposed interaction and implementation

1. Enter an activity immediately using its cached title/art. Connect in the
   background. Show controls as soon as playback state arrives; never wait on an
   image download. Starting an activity and reopening its screen are different:
   reopening must not rerun power/input actions.
2. Prefer a persistent notification-capable connection, with one worker owning
   the existing non-`Sync` client. Keep notification waits short and prioritize
   queued button commands. Use notifications to reconcile play/pause/seek/stop,
   plus a proposed 5–10 second resync and immediate reconnect refresh. Advance
   displayed progress from a monotonic clock between observations; stop advancing
   when paused/disconnected. Tune the interval against the real player.
3. Reuse the Hue lesson: tag media and commands with generations. A stale response
   must not undo a newer command, replace the next item's artwork, or reopen a
   dismissed sheet. Show a seek preview while dragging; send on release, coalescing
   repeated physical seeks. Keep command acknowledgements separate from media
   transitions and report failures without implying success.
4. Keep play/pause initially focused. D-pad moves through controls; OK activates.
   A visible TV-controls mode routes D-pad/OK to Kodi. Back first closes a sheet,
   then returns to the room without stopping the film. Volume follows an explicit
   activity audio target; initially it can be Kodi. Channel keys can move between
   known chapter timestamps while this activity owns input; they must not also
   invoke room scenes. Exiting clears transient feedback.
5. Implement static artwork plus a precomputed dark gradient, avoiding live blur
   and continuously animated backgrounds. Decode off the GUI thread; publish
   Slint images on the UI thread. Bound downloads, dimensions and caches. A
   480 × 800 RGBA image is about 1.46 MiB; two are about 2.93 MiB before decoder
   working memory. Repaint the progress region at 1 Hz; animate only interaction
   and short transitions. Retain the repository's measured cached-RAM renderer;
   this screen does not justify a speculative GPU/kernel rewrite.

Kodi documents [notification-capable transports and runtime introspection](https://kodi.wiki/view/JSON-RPC_API).
The worker, cache, resync interval and key mapping above are implementation
proposals, not claims that Kodi mandates them.

Build the first slice with play/pause, timeline, fanart/clearlogo, explicit source
selection and error states; then add capability-gated chapters and streams.
Validate on-device with bright/missing artwork, long titles, rapid seeks,
item changes during downloads, reconnects and Back during delayed replies.

## Asset attribution

- `assets/fanart.png`: unmodified frame `graded_edit_final_04500.png`,
  *Tears of Steel*, © Blender Foundation, [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/),
  [original frame](https://media.xiph.org/tearsofsteel/tearsofsteel-1080-png/graded_edit_final_04500.png),
  [source license](https://media.xiph.org/tearsofsteel/README.txt). CSS crops and
  darkens its presentation; it is sample fanart, not a live video feed.
- The transparent title SVG is an original typographic stand-in for a clearlogo,
  not the official movie logo. Production displays the user's Kodi artwork.
- Inter is copied from `assets/inter` in this repository, including its OFL license.
- Lucide SVG paths are copied from the repository's licensed catalog; see
  `assets/LUCIDE-LICENSE`.

## Prototype validation

With the local server running and the repository's existing Playwright install
at `build/webui-review/node_modules`, run:

```sh
node docs/mockups/kodi-activity/check.mjs
```

Verified all three concepts: play/pause, relative seeks, chapter selection,
audio selection, Back/return without stopping, idle TV input, disconnected and
live states, unavailable chapter API, complete chapter rows and mobile overflow.
The check also refreshes the four native-resolution PNG exports. Device runtime,
real media compatibility and physical-key behavior remain implementation work.
