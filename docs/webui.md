# The config web UI

How the house gets described: `couch-confd`, a static binary on the remote that
owns `/opt/couch/config.json`, and `couch-web`, the page it serves to a phone on
the same LAN.

> **There is no authentication.** Anything that can reach the port can rewrite
> the house. This is a deliberate, documented gap - see [No
> authentication](#no-authentication) - and it has to be closed before this is
> in an image anyone else installs.

## Three crates, three targets

```
model/couch-model     the types, and the rules about them   armv7 + wasm32 + host
daemon/couch-confd    the HTTP server and the REST API      armv7 musl (and host)
web/couch-web         the page, as a WASM bundle            wasm32
```

`couch-model` is the reason the other two agree. `Config`, `Area`, `Room`,
`Device`, `Scene` and `Activity` are defined once, with their `serde`
derivations, and both sides import them: the daemon deserialises the file into
the same structs the browser edits, and the icon and device-kind pickers are
built from the model's own `ALL_ICONS` and `ALL_DEVICE_KINDS` rather than from a
copy that can drift. Renaming a field is a compile error on both sides instead
of a JSON key that silently stops matching.

It is `no_std` + `alloc`. Nothing in it needs to be - it is types, `Vec` and
`String` - but staying off `std` costs nothing and means the wasm build has no
reason to pull one in.

Each crate is its own cargo workspace, for the same reason `clients/` and `ui/`
are separate: one lockfile that has to satisfy `armv7-unknown-linux-musleabihf`,
`wasm32-unknown-unknown` and the host at once is a lockfile where one consumer's
dependency is the other's problem.

### The daemon is not a hub

`couch-confd` never talks to a light or a TV. It reads and writes one JSON file
and serves the page that edits it, which is what lets it stay a megabyte and why
a crash in it cannot take the remote's UI with it. It also does not replace
`stage2/portal.sh`: the setup portal answers "which network should this join",
runs on port 80 out of an AP the device hosts itself, and has to work before
there is any network at all. This answers "what is in your house", and needs one.

## Why these libraries

**tiny_http, not axum.** axum is the better library for a service and the wrong
one for this device. It arrives with tokio, hyper, tower and their transitive
halves - a reactor, a work-stealing scheduler and a connection pool - to answer
a handful of requests from one browser. tiny_http is blocking sockets and a
request queue, four dependencies deep; a fixed pool of four threads covers the
six connections a browser opens to one origin. It also contains no C, which is
what keeps the static musl cross-build a one-liner (`rust-lld` as the linker, no
cross-gcc, no Docker, no sysroot).

**Leptos CSR, not a JS framework.** The page shares `couch-model` with the
daemon, which is the whole argument: any JavaScript framework means maintaining
a second description of the same types, in a language where the compiler cannot
check it against the first. Leptos in client-side-rendering mode is the smallest
way to get that - no server half, no hydration, no islands, one wasm file and a
loader. The cost is measured below, and 387 KB of wasm is real; against it, the
config model has exactly one definition and the icon list on the phone cannot
disagree with the icon list on the device.

**Leptos' own signals, no router crate.** `leptos_router` would add a matcher, a
nested-route tree and a set of macros; `route.rs` is one enum, two string
functions and a `popstate` listener, because there are seven screens. The URL is
real - `/rooms/kitchen` reloads to that room - because on a phone Back means "up
one level", and without history entries Back leaves the app from the first
screen a user drills into.

**No CSS framework and no CDN.** The remote serves this over its own WiFi, which
may have no route to the internet at all. Everything is in the bundle: one
hand-written stylesheet, the system font stack, and an inline SVG favicon (a
browser asks for `/favicon.ico` on every load, and a 404 per load for want of
300 bytes is silly). The palette is `stage2/www/index.html`'s, so the setup
portal and the config UI look like the same device talking.

**The UI is baked into the binary.** `daemon/couch-confd/build.rs` walks
`web/couch-web/dist` and writes an `include_bytes!` table. A deploy is then
`scp` of one file, and the API and the page that calls it are versioned
together. Anything that has to exist on disk at a fixed path is something a
partial copy can leave half-present, and a half-present web root serves a blank
page with no error anywhere. A build with no `dist` is not an error - it is the
state of every fresh clone - and the daemon serves a page saying what to run.

## Two conventions hold the API together

**Every mutating response is the whole config.** A house is a few kilobytes. A
client that re-reads it after every edit cannot drift out of step with the
server, and the alternative - patching a local copy from partial responses - is
where "the room disappeared until I reloaded" bugs come from. The browser holds
exactly one piece of state, `RwSignal<Option<Config>>`, and every screen is a
pure function of it.

**Every response carries `X-Couch-Revision`.** The store increments a counter on
each accepted write. Mutations accept `If-Match: <revision>`; a mismatch is a
409 rather than a silent overwrite. Creates additionally answer with
`X-Couch-Created: <id>`, because the id is the one thing the client cannot work
out for itself.

Writes are validated before they are kept. `Store::mutate` applies the edit to a
*copy*, runs `Config::validate`, and only then replaces the in-memory config and
writes the file - temp file in the same directory, `fsync`, `rename`, `fsync` of
the directory. A rejected edit changes nothing at all, and a battery pull cannot
leave a present-but-empty config behind. A *corrupt* file at startup is an
error, not a reset: overwriting somebody's house because one brace is missing is
data loss with extra steps. A *missing* file is first boot, and gets the seed
house.

## The API

Everything under `/api`; everything else is the page. Bodies and responses are
JSON. `{id}` is a slug like `living-room`.

| Method   | Path                                   | What it does |
|----------|----------------------------------------|--------------|
| `GET`    | `/api/health`                          | service, version, schema version, config path, revision, embedded asset count |
| `GET`    | `/api/meta`                            | the vocabularies the editor builds its pickers from: icons, device kinds, integrations, activity kinds |
| `GET`    | `/api/config`                          | the whole document |
| `PUT`    | `/api/config`                          | replace it wholesale (the revision stays the server's to hand out) |
| `POST`   | `/api/config/reset`                    | back to the seed house |
| `GET`    | `/api/areas`                           | the areas, in order |
| `POST`   | `/api/areas`                           | `{name, icon?}` |
| `GET`    | `/api/areas/{id}`                      | one area |
| `PUT`    | `/api/areas/{id}`                      | `{name, icon?}` - name and icon travel together so saving one cannot clear the other |
| `DELETE` | `/api/areas/{id}`                      | the area only; its rooms, scenes and activities belong to the house |
| `PUT`    | `/api/areas/{id}/rooms`                | `["living-room", ...]` - replace the list, which is also how it is reordered |
| `POST`   | `/api/areas/{id}/rooms`                | `{room}` to attach an existing one, `{name, icon?}` to create and attach |
| `DELETE` | `/api/areas/{id}/rooms/{room}`         | detach; the room stays in the house |
| `PUT`    | `/api/areas/{id}/scenes`               | replace/reorder |
| `POST`   | `/api/areas/{id}/scenes`               | `{scene}` or `{name, icon?}` |
| `DELETE` | `/api/areas/{id}/scenes/{scene}`       | detach |
| `PUT`    | `/api/areas/{id}/activities`           | replace/reorder the activity strip |
| `POST`   | `/api/areas/{id}/activities`           | `{activity}` to attach, or `{name, room, kind?, source?}` to create and attach |
| `DELETE` | `/api/areas/{id}/activities/{act}`     | take it off this area's strip |
| `GET`    | `/api/rooms`                           | every room in the house |
| `POST`   | `/api/rooms`                           | `{name, icon?}` |
| `GET`    | `/api/rooms/{id}`                      | one room, with its devices |
| `PUT`    | `/api/rooms/{id}`                      | `{name, icon?}` |
| `DELETE` | `/api/rooms/{id}`                      | and every reference to it: area lists, its devices, activities anchored to it, scene steps naming those devices |
| `GET`    | `/api/rooms/{id}/devices`              | the room's devices |
| `POST`   | `/api/rooms/{id}/devices`              | `{name, kind?, icon?, integration?}` |
| `PUT`    | `/api/rooms/{id}/devices/{device}`     | the whole device; the path names it, so a body with a different id cannot move it |
| `DELETE` | `/api/rooms/{id}/devices/{device}`     | and every scene step and activity step pointing at it |
| `GET`    | `/api/scenes`                          | every scene |
| `POST`   | `/api/scenes`                          | `{name, icon?}` |
| `GET`    | `/api/scenes/{id}`                     | one scene |
| `PUT`    | `/api/scenes/{id}`                     | `{name, icon?, steps}` |
| `DELETE` | `/api/scenes/{id}`                     | and its references from every area |
| `GET`    | `/api/activities`                      | every activity |
| `POST`   | `/api/activities`                      | `{name, room, kind?, source?}` |
| `GET`    | `/api/activities/{id}`                 | one activity |
| `PUT`    | `/api/activities/{id}`                 | `{name, kind?, room, source?, steps}` |
| `DELETE` | `/api/activities/{id}`                 | and its references from every area |

Status codes, and what each actually means here:

| Code  | When |
|-------|------|
| `200` | including every mutation, whose body is the new config |
| `400` | unparseable body, or a create missing a field it cannot default (an activity with no room) |
| `404` | no such route, or the thing named is not there any more - the usual cause is a second phone that deleted it |
| `405` | a write to the web root |
| `409` | `If-Match` no longer matches |
| `413` | body over 512 KB. A whole house is a few KB; the cap is there so a stuck client cannot make a device with 1 GB and no swap allocate |
| `422` | the edit would leave the config invalid. The body carries `problems: [{at, message}]`, each naming a path like `areas[1].rooms[0]` |
| `500` | the write failed - a full partition |

Static assets: `GET`/`HEAD` only. Anything not found and without a file
extension falls back to `index.html`, because the frontend routes in the
browser and a reload on `/areas/kitchen` must reach the app. A path *with* an
extension that misses is a genuine 404 - serving HTML where a script was asked
for produces a syntax error in the console and nothing that says what really
happened. Trunk fingerprints its output, so those get
`Cache-Control: public, max-age=31536000, immutable`; `index.html` names them
and is `no-cache`.

```console
$ curl -s localhost:8090/api/health
{"config_path":"/opt/couch/config.json","embedded_assets":4,"revision":26,
 "schema_version":1,"service":"couch-confd","version":"0.1.0"}

$ curl -si -X POST localhost:8090/api/areas -d '{"name":"Loft"}' | head -4
HTTP/1.1 200 OK
Content-Type: application/json
X-Couch-Revision: 27
X-Couch-Created: loft
```

## Sizes, measured

Release builds, `opt-level="z"` and LTO for the wasm, `opt-level="s"` for the
daemon, both stripped. `wasm-opt -Oz` runs as part of the trunk build.

| Artefact | Bytes | gzip -9 |
|----------|------:|--------:|
| `couch-web-*_bg.wasm` | 396,446 (387 KB) | 158,913 |
| `couch-web-*.js` (wasm-bindgen loader) | 38,393 | 7,068 |
| `style-*.css` | 6,732 | 2,287 |
| `index.html` | 2,201 | 1,286 |
| **bundle total** | **443,772 (433 KB)** | **169,554 (166 KB)** |

`tools/build-webui.sh` prints this table at the end of every run. The last few
bytes wobble between builds because trunk's content hashes are not always the
same length, and `index.html` names two of them.

| Binary | Bytes |
|--------|------:|
| `couch-confd`, `armv7-unknown-linux-musleabihf`, static musl | 1,172,472 (1.12 MB) |
| `couch-confd`, host (macOS arm64) | 1,098,560 (1.05 MB) |

The ARM binary carries the whole bundle inside it, so the daemon is about
730 KB of code and 433 KB of page. The daemon does not compress responses: the
bundle would go over the wire at a third the size gzipped, which is the obvious
next saving if first load on the remote's own AP ever feels slow.

## Building and running, locally

```sh
tools/build-webui.sh          # frontend, then the armv7 daemon that embeds it
tools/build-webui.sh --host   # the same, for this machine
tools/run-webui.sh            # serve it on http://127.0.0.1:8090
```

`build-webui.sh` exists because the order matters and cargo cannot express it:
`build.rs` bakes `web/couch-web/dist` into the binary, so trunk has to have run
first or the daemon ships the placeholder page. It prints every artefact's size
at the end.

Prerequisites, once:

```sh
rustup target add wasm32-unknown-unknown armv7-unknown-linux-musleabihf
cargo install --locked trunk
```

`run-webui.sh` serves the page from `dist/` with `--www` rather than from the
embedded copy, so a CSS change is a `trunk build` away rather than a daemon
rebuild, and points the daemon at `build/couch-config.json` so the loop cannot
touch a real house. Both are overridable:

```sh
COUCH_CONFD_ADDR=0.0.0.0:8090 COUCH_CONFIG=/tmp/house.json tools/run-webui.sh
```

The daemon takes the same three settings as flags or environment
(`--addr`/`COUCH_CONFD_ADDR`, `--config`/`COUCH_CONFIG`, `--www`/`COUCH_WWW`);
`--help` prints them.

For the markup-and-CSS loop, `trunk serve` rebuilds on save and proxies `/api`
to a daemon you start yourself (see `web/couch-web/Trunk.toml`):

```sh
daemon/target/release/couch-confd --config /tmp/couch.json --addr 127.0.0.1:8090
cd web/couch-web && trunk serve      # http://127.0.0.1:8080
```

Tests: `cd model && cargo test` covers the model, its validation rules and the
seed; `cd daemon && cargo test` covers the store's atomic write, the revision
gate, path traversal and the asset fingerprinting.

## Deploying to the device

The binary is over a megabyte, so `tools/push.py` - 512 bytes a line over the
USB serial shell - is the wrong tool. Copy it over ssh, which means the device
is already on WiFi with sshd running (`stage2/sshd.sh` only starts it once a key
or password has been enrolled through the setup portal).

`sshd` runs inside the Alpine chroot, so `/opt/couch` in an ssh session is the
directory the initramfs sees as `/mnt/alpine/opt/couch`.

```sh
tools/build-webui.sh
IP=192.168.1.79                        # COUCH_IP in tools/screenshot.sh
KEY=~/.ssh/couch_dev
SSH="ssh -i $KEY -o IdentitiesOnly=yes"

scp -i $KEY daemon/target/armv7-unknown-linux-musleabihf/release/couch-confd \
    root@$IP:/opt/couch/couch-confd.new
$SSH root@$IP '
    killall couch-confd 2>/dev/null
    mv /opt/couch/couch-confd.new /opt/couch/couch-confd
    chmod 755 /opt/couch/couch-confd
    nohup /opt/couch/couch-confd </dev/null >/tmp/confd.log 2>&1 &
'
```

Copied beside its destination and moved into place, because a half-written
binary at the real path is what the next restart runs.

That sequence has been run against the HA100. What it looked like: assets served
with the right types and a real `Content-Length` (396446 for the wasm, byte-exact
on arrival), a create round-tripping to `/opt/couch/config.json` and coming back
after a restart at the revision it was left at, and 740 kB RSS alongside a
running `couch-gui` - the two do not contend for anything, since the daemon
touches neither the framebuffer nor the keypad. Then
`http://192.168.1.79:8090` from a phone on the same network.

Nothing starts it at boot yet. When that is wanted, it belongs in
`stage2/stage2.sh` next to the block that starts `couch-gui`, and after the
network is up - it is useless without one:

```sh
CONFD="$(dirname "$0")/couch-confd"
if [ -n "$IP" ] && [ -x "$CONFD" ]; then
    ( while true; do "$CONFD" >>/tmp/confd.log 2>&1; $BB sleep 2; done ) &
    echo "= couch-confd on $IP:8090"
fi
```

**Do not add that block to a shipped image while the server is unauthenticated.**

## No authentication

`couch-confd` has no authentication, no authorisation and no transport
security. Every endpoint above is open to anything that can open a TCP
connection to port 8090:

* any device on the LAN can read the whole configuration - room names, device
  names, Home Assistant entity ids, Kodi hostnames and ports;
* any device on the LAN can rewrite or delete all of it, with no audit trail;
* traffic is plain HTTP, so anything on the path can read and alter it;
* there are no CSRF defences, so a page in a browser on the same network can
  issue writes with a form post or a `fetch`;
* nothing rate-limits anything.

This is a deliberate gap and not an oversight. The daemon exists to be driven
from a phone on the same network as the remote, and every mechanism that would
close the gap - a password to store, a token to enrol, a certificate to trust -
needs a decision about where the credential comes from and how a factory-reset
device gets a new one. That decision belongs with the setup portal, which
already owns enrolment: it is where the SSH key and root password are
established today, gated on a physical button press (`stage2/confirm.sh`).

**It must be closed before this ships in an image.** The likely shape, reusing
what exists:

1. A token generated on first boot, stored in `/opt/couch/confd.token`, shown on
   the panel as a QR code the way the setup SSID already is (`ui/couch-gui/src/qr.rs`).
2. The browser exchanges it once for a cookie; the daemon checks that cookie on
   every `/api` request and every asset.
3. Same-origin checks on mutations, since the token in a cookie is otherwise
   CSRF-able.
4. Bind to the LAN interface rather than `0.0.0.0` where that is meaningful, and
   consider requiring the physical confirm press for a *first* pairing, which is
   the pattern the portal already sets.

Until then: run it on a network you trust, and stop it when you are done. The
`--help` text says so, `api.rs` says so at the top, and this section is what
they point at.

## Known gaps

* **The client never sends `If-Match`.** The store implements optimistic
  concurrency and the daemon honours the header, but `web/couch-web/src/api.rs`
  does not set it, so two phones editing at once still last-write-wins. Wiring
  it is a few lines; what it needs first is a decision about what the second
  phone should *see* - a 409 for your own double-tap would be worse than the
  race it prevents.
* **No compression.** See the size table.
* **Free-text commands.** A scene step's `command` is a string (`on`,
  `input:hdmi2`, `dim:30`) because what a device accepts is the integration's
  business. The editor suggests, and validates nothing.
* **Device ids stutter.** A device created as "Cellar light" in the Cellar
  becomes `cellar-cellar-light`: the id is built from the room's id and the
  name, which keeps ids unique and readable everywhere else.
* **No schema migration.** `SCHEMA_VERSION` is 1 and the daemon refuses a file
  from the future. There is no code to upgrade an older file, because there is
  no older file yet.

## What `couch-gui` would need

Not done here - `ui/couch-gui/` is untouched - but this is what connects it.

The GUI's hard-coded `areas` vector in `main.rs` is now shaped exactly like the
model: an `Area` with a name, its own `activities`, its `rooms` and its
`scenes`. That was the reconciliation this work had to make. `couch-model`
originally derived an area's activity strip from room membership, which cannot
reproduce the mock-up - WHOLE HOME shows three activities in rooms that host
five, DOWNSTAIRS shows five across three rooms, and no derivation gives both.
An area now *lists* its activities the way it already listed its scenes, and
`Config::seed()` reproduces the mock-up's counts exactly: 3/1/5/0 activities and
5/3/3/4 scenes, asserted by a test in `model/couch-model/src/lib.rs`.

To render a real config, `couch-gui` would:

1. Take `couch-model` as a dependency (it builds for armv7 already; the GUI is
   the third consumer the crate was written for) and read
   `/opt/couch/config.json` at startup, falling back to `Config::seed()` when
   there is no file - which is what it effectively shows today.
2. Build each `RoomRow` from a `Room`: `device_summary()` gives `"5 devices"`
   and `device_detail()` gives `"Kodi, Hue, LG C3"`, which are the two strings
   the mock-up hard-codes. `effective_icon().glyph_index()` gives the `glyph`
   field, which is why the config stores an icon *name* and not the index -
   inserting one icon into `tools/mkuiicons.sh` would otherwise renumber every
   room in the file.
3. Build each `SceneCell` from `Config::scenes_in_area`.
4. Build the activity strip from `Config::activities_in_area`, which returns the
   configured activities in the area's own order. `LiveActivity`'s `title`,
   `source` and `place` are *runtime* state - "Paused - Andrei Rublev" is not
   something anybody configures - so those come from whatever hub daemon
   eventually tracks what is playing. The configured `Activity` supplies the
   name, the room, the source device and `kind.glyph_index()`; `active` on a
   `SceneCell` is the same kind of live state.
5. Reload when the file changes, so an edit on the phone appears on the remote
   without a restart. `couch-confd` writes by rename, so a watch on the
   directory is the reliable form.

The counts in the seed are what make step 4 checkable: a first cut can be held
against the screen it replaces.
