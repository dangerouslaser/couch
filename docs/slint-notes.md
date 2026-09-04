# Slint on this device

Notes that cost time to discover. Slint 1.17.1, software renderer, no `std`.

## The feature set, and why

```toml
slint = { version = "1.17", default-features = false, features = [
    "compat-1-2", "renderer-software", "libm", "unsafe-single-threaded",
] }
```

No `std`, deliberately: that feature turns on system font discovery, which pulls
`yeslogic-fontconfig-sys` - a C library wanting pkg-config and a cross sysroot,
for a device with no font system. This is the configuration Slint's
microcontroller targets use.

## Vector paths: available, but opt-in on two crates

`Path` works in the software renderer. Getting there is not obvious, and two
wrong conclusions came out of it before the right one:

- It is **not** gated on `std`. What is std-gated is the *re-export* of
  `PathData` from `i-slint-core`, which is not what generated code needs.
- The software renderer **does** implement `draw_path`, behind its own `path`
  feature (pulling `zeno` and `lyon_path`).

The feature must be enabled on **both** crates, and the `slint` facade forwards
it from neither, so they have to be named directly:

```toml
i-slint-core = { version = "1.17", default-features = false, features = ["path"] }
i-slint-renderer-software = { version = "1.17", default-features = false,
                              features = ["path", "libm"] }
```

Enabling it on `i-slint-core` alone gives `error: not all trait items
implemented, missing: draw_path` - the trait grows the method while the
renderer's implementation stays compiled out. That error reads as "paths are
unsupported"; it means "half configured".

Note `lyon_path` appears in `cargo tree` regardless: `i-slint-compiler` uses it
at build time. That is host-side and does not reach the target binary.

We are **not** using this today. Measured against rasterised alpha masks it cost
186KB and about 10% more per frame, and the design is authored so every glyph is
a rectangle, a circle or a triangle - so the capability had nothing else to pay
for itself with. If a circular scrubber or an arc meter ever lands, turn it on
and `lucide-slint` becomes nearly free on top.

## Glyph embedding happens at compile time

Glyphs are embedded from strings that appear in `.slint`. A character that only
ever exists in a runtime string is never embedded and renders as a blank gap -
which is what happened to every `·` separator when the subtitles were composed
in Rust. Compose such strings in `.slint` with the separator as a literal.

The compiler seeds that set before it looks at any literal, with `a-z`, `A-Z`,
`0-9`, `●`, `…`, space, and the punctuation `!"#$%&'()*+,-./:;<=>?@[\]^_{|}~`
(see `passes/embed_glyphs.rs`). So the rule bites for anything outside that: an
accented letter, a currency symbol, an arrow - and, of printable ASCII, the
backtick, which is the one character the seed leaves out.

The cost of a size is the whole set at that size, in every embedded face, and
it goes as the square of the size. Measured against this UI: 32KB at 20px, 40KB
at 26px, 162KB at 48px, 327KB at 72px. Reusing a size the UI already asks for
is free; picking one two pixels away is not.

`SLINT_FONT_PATH` and `SLINT_DEFAULT_FONT`, set in `build.rs`, choose the face.
Ship every weight the UI asks for: `font-weight: 600` against a single Regular
face gets synthesised rather than resolved.

## `.slint` can grow a string but not shrink one

The string members are `is-empty`, `character-count`, `is-float`, `to-float`,
`to-lowercase`, `to-uppercase`. There is no substring, no slice, no index. `s +
"a"` is available; taking that `a` off again is not, which means a backspace
cannot be written in `.slint` at all.

`TextInput` is the way out, and it is worth reaching for rather than working
around: it owns a real buffer, a cursor, `input-type: password`, and
scroll-to-cursor. It can be driven without ever being typed into - its
`key-pressed` callback runs *before* its own handling, so a D-pad's arrows can
be claimed before it sees them, and a delete is `set-selection-offsets(n-1,
big)` then `cut()`. `cut` copies to the clipboard first, but
`Platform::set_clipboard_text` defaults to a no-op and this device implements
none, so nothing escapes. Offsets are bytes, and are clamped rather than
checked: an offset past the end lands at the end.

Set `text-cursor-width`, `color`, `selection-background-color` and
`selection-foreground-color` explicitly. Anything left unset is filled in from
StyleMetrics and the palette by a compiler pass, which pulls the widget style
into a binary that has no widgets in it.

## A component's root cannot see `parent`

A component's own root has no parent at definition time, so `parent.width` there
is an error. Either measure off `root` (its own geometry, set by the caller) or
take the value as a property. The focus ring takes the box it rings; the volume
overlay is positioned by its caller.

## DirtyRegion holds three rectangles

```rust
/// The maximum number of rectangles that can be stored in a DirtyRegion
pub const MAX_COUNT: usize = 3;
```

Past three, `add_box` merges new boxes into whichever rectangle grows least,
"simplified by being bigger than the actual union". A state change touching six
elements collapsed into three bands covering 80% of the panel, and the renderer
faithfully redrew all of it - a partial update that cost more than a full
redraw.

The rule that follows: **make a state change move one element rather than
restyle many**, and never express as geometry what can be expressed as colour
(changing `border-width` changes geometry; changing `border-color` to
transparent does not). Replacing a per-row selected state with one moving
highlight took the dirty region from 80% to 7-14% and the frame from 8.3ms to
2.2ms.

## What things actually cost here

Measured on the panel, 480x800:

| | cost |
|---|---|
| memcpy RAM -> framebuffer | 1.3 ms |
| alpha blend, full screen | 11 ms |
| hub focus move (71% dirty) | ~22 ms |
| hub page slide, average | ~10 ms |
| hub page slide, worst | 22-25 ms |

Anti-aliased rounded rectangles dominate. This content runs at roughly 80ns per
pixel against 14ns for flat cards, because a row card, a 26px icon disc and a
focus ring are all rounded and all anti-aliased. Any estimate taken from a
simpler screen will be optimistic by several times - measure the real thing.

`COUCH_REGION=1` prints what the renderer marked dirty each frame. A frame that
costs far more than its content suggests is almost always claiming a larger
region than it needs, and that is invisible without it.

## Dirty regions: what costs a frame, measured on the HA100

`COUCH_REGION=1` prints every rectangle the renderer marked dirty, with
geometry. Read it before optimising anything: three separate theories about a
stutter here were wrong, and the rectangles settled it in one run each.

The case: the focus ring appeared to skip. It did not - the animation frames
were a tidy 12% of the screen at ~2.2ms, stepping 1-2px. Every focus move also
emitted **one 452x654 frame, ~23ms** immediately before the animation started.
The move hitched, then glided.

Things that turned out **not** to be the cause, each disproved by measurement:
the animation itself (removing it left the frame exactly where it was); the
ring's clamping; an `animate` block whose duration read a property; a layout
feedback loop in the row window; the dot indicators; focus arriving as a bound
`in property` versus an imperative function call; focus owned by the component
root versus a zero-size child; and the ring living inside a clipped, scrolling
subtree.

What it was: **three focus rings, one per section.** Each had a `focused`
expression over `focus-row`, so all three re-evaluated on every move - including
moves between rooms, where two of them answer false before and after. Two of
those rings sat at the very top and the very bottom of the pane, and
`DirtyRegion::MAX_COUNT` is 3: past three rectangles Slint merges, and the merge
of "something at the top, something at the bottom" is the whole pane.

Two lessons worth keeping:

**Slint propagates on dependency, not on value.** Hoisting the expressions into
named `bool` properties changed nothing. If an element must not repaint, it must
not *depend* on the changing property at all - naming the expression is not
enough.

**Count your dirty rectangles.** Past three they merge into a bounding box, so
two cheap changes far apart on screen cost more than one expensive change. The
fix was one ring for the whole pane, positioned over whichever cell or row has
focus: one reader, one band.

Bisect by disabling readers, not by reasoning about them. Setting each
`focused:` to a constant `false` in turn took twelve moves from thirteen
oversized frames to one and named the culprit in a single run, after several
hours of plausible theories had not.

Section offsets for that single ring are arithmetic over the same furniture
heights the row window uses, not read back off laid-out elements - reading the
layout back is its own trap, see the note on the row window above.
