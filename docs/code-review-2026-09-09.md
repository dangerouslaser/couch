# Integration and activity code review — 2026-09-09

Scope: shared configuration, connection APIs, activity controls, physical input,
and the new Denon client. This was a source review with Rust tests, browser
checks and HA100 protocol fixtures, not a kernel/GPU performance audit.

## Fixed during review

1. **Control sockets outlived activities.** The mapping worker blocked waiting
   for its next key and retained Denon/TV clients after navigation. It now checks
   the activity generation while idle and drops those clients on exit. A TCP
   fixture verifies EOF without sending another key. This also prevents reuse
   of an old pairing when reopening an activity after connection changes.
2. **Older clients could erase bindings.** Activity updates previously treated
   a missing `buttons` field as an empty list. The API now preserves bindings
   when the field is omitted; current clients send an explicit array to clear
   mappings. The API regression test covers preservation, explicit removal,
   invalid commands and persistence.

The browser verification also caught and fixed the initial activity-save path
omitting bindings entirely. Short/long exclusion and cancellation on navigation
are covered by both timing tests and physical evdev fixtures.

## Approved refactors implemented

1. **Independent command ownership.** Mapped devices now have separate bounded
   workers. Kodi, webOS and Denon share the daemon's per-endpoint control service
   across GUI and HTTP requests, with queue metrics and explicit lease cleanup.
   Tests prove a stalled receiver cannot block another and that IPC/local
   consumers share one socket. Existing Hue/HA push subscriptions are retained.
2. **Shared configuration snapshots.** Controllers share a validated immutable
   `Arc<Config>`. A background watcher publishes changes; invalid files preserve
   the last valid snapshot. Room and controller reads no longer parse files on
   the rendering thread. Logs separate snapshot load costs from rendering.
3. **Typed functions and discovered choices.** Validation and activity execution
   use the shared `Function` enum. Existing persisted commands remain compatible;
   Denon/webOS inputs and webOS apps can be discovered for button mappings.
4. **Input/navigation extraction.** `input.rs` owns touch/wake policy and menu/mic
   hold state. `navigation.rs` owns area/settings framebuffer transitions,
   preserving their drawing order and timing. The main loop remains responsible
   for coordinating hardware and controllers.

See [control-service.md](control-service.md) for ownership, limits and diagnostics.

The current separation between pure model validation, blocking Rust clients and
Slint rendering is sound. Keep native hardware validation distinct from host
unit tests: a passing host build cannot prove panel wake or physical key routing.

Validation after refactors: 210 Rust tests passed across model, clients, daemon and GUI. Workspace-wide `cargo fmt --check` is not clean; formatting debt remains in existing and expanded modules. New standalone modules were formatted, and `git diff --check` passes.

Hardware validation after refactors: short/long activity mappings and held volume
repeat passed through both standalone and daemon IPC control, including pending
hold cancellation on exit. The webOS fixture passed input/app/sound selection,
playback controls, physical TV keys and animated touch exit. An isolated charging
fixture verified live timezone/format updates, clock persistence beyond the off
timeout, button/touch wake without issuing a TV command, and undocking. Snapshot
loads measured 582–761 us in these small fixture configurations; these are load
costs, not a claimed end-to-end speedup. Activity slides retained 11 frames.
