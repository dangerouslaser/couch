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

## Recommended next refactors

1. **One command owner per connection.** `activity_buttons.rs` currently uses
   one bounded worker for all mapped devices. A slow TV connection can delay a
   receiver command until its 750 ms queue deadline expires. Independent queues
   keyed by connection would isolate failures and give each provider clear
   socket ownership across direct control, activities and web requests. Add
   queue-delay/drop metrics before changing the architecture.
2. **A shared, revisioned configuration snapshot.** Activity, light, scene and
   clock controllers independently reread/parse configuration; some reads occur
   on the rendering thread. Load once after a revision change and distribute an
   immutable `Arc<Config>`. Measure activity-open/render timings before and after.
3. **Typed provider actions and capabilities.** The shared function catalog
   prevents arbitrary RPC execution, but string commands are still translated
   again in several controllers. A typed action layer would make unsupported
   cases explicit and reduce drift between web choices and execution. Extend it
   with discovered input/app choices rather than adding more string prefixes.
4. **Extract input and navigation ownership from `main.rs`.** Wake handling,
   microphone/menu semantics, activity overrides, focus and framebuffer slides
   now interact in one large loop. Preserve the existing evdev fixtures while
   separating these responsibilities; do not replace working hardware paths in
   one wholesale rewrite.

The current separation between pure model validation, blocking Rust clients and
Slint rendering is sound. Keep native hardware validation distinct from host
unit tests: a passing host build cannot prove panel wake or physical key routing.

Validation after fixes: 202 Rust tests passed across model, clients, daemon and GUI. Workspace-wide `cargo fmt --check` is not clean; formatting debt remains in existing and expanded modules. New standalone modules were formatted, and `git diff --check` passes.
