#!/usr/bin/env python3
"""The two hub glyphs that a Rectangle cannot express.

The design specifies every glyph as a primitive, and four of the five are:
a square, a circle, and a rounded-top rect are all Rectangles (Slint has
per-corner radii). A square on its point and a play triangle are not, and
Slint's Path element is unavailable here - PathData is gated behind Slint's
"std" feature, which we cannot enable because it pulls in fontconfig.

So these two are rendered once, here, as alpha masks. They are tinted at
runtime with Image.colorize, so the accent colour still lives in the theme
rather than in a pixel.
"""
import pathlib
from PIL import Image, ImageDraw

OUT = pathlib.Path("ui/couch-gui/assets")
OUT.mkdir(parents=True, exist_ok=True)
SS = 8  # supersample, then downscale: these are 20px shapes with 2.5px strokes


def save(img, name, size):
    img.resize(size, Image.LANCZOS).save(OUT / name)
    print(f"  {name}: {size[0]}x{size[1]}")


# Study: a square on its point, 2.5px stroke, in a 20x20 box.
d = Image.new("RGBA", (20 * SS, 20 * SS), (0, 0, 0, 0))
ImageDraw.Draw(d).polygon(
    [(10 * SS, 2 * SS), (18 * SS, 10 * SS), (10 * SS, 18 * SS), (2 * SS, 10 * SS)],
    outline=(255, 255, 255, 255), width=int(2.5 * SS))
save(d, "glyph-diamond.png", (20, 20))

# Kodi activity: a filled play triangle, 14x18.
t = Image.new("RGBA", (14 * SS, 18 * SS), (0, 0, 0, 0))
ImageDraw.Draw(t).polygon(
    [(0, 0), (14 * SS, 9 * SS), (0, 18 * SS)], fill=(255, 255, 255, 255))
save(t, "glyph-play.png", (14, 18))
