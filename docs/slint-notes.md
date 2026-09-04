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

`SLINT_FONT_PATH` and `SLINT_DEFAULT_FONT`, set in `build.rs`, choose the face.
Ship every weight the UI asks for: `font-weight: 600` against a single Regular
face gets synthesised rather than resolved.

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
