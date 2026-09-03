#!/usr/bin/env python3
"""Generate the media-scene test assets.

Synthetic rather than downloaded, so the repo carries no third-party artwork,
but built to have the properties that matter for the measurement: full-bleed
photographic gradients with enough entropy that nothing compresses or blits
trivially, and a logo with a real alpha edge to composite.

Emits two forms of each:
  *.png       for Slint, whose TargetPixel handles the channel order
  *.rgba      raw R,G,B,A for couch-gui. NOT swapped: LVGL blits image bytes
              through unchanged, so stored R,G,B,A they reach the panel as
              R,G,B,A, which is the order it wants. couch_rgb()'s swap applies
              only to colours built from r/g/b components.
"""
import math, pathlib, random
from PIL import Image, ImageDraw, ImageFilter

W, H = 480, 800
OUT = pathlib.Path("assets")
OUT.mkdir(exist_ok=True)


def backdrop(name, base, accents, seed):
    rnd = random.Random(seed)
    img = Image.new("RGB", (W, H), base)
    d = ImageDraw.Draw(img)
    # Broad colour field first, then soft blobs, then grain: roughly the
    # frequency distribution of a real still.
    for i in range(H):
        t = i / H
        d.line([(0, i), (W, i)],
               fill=tuple(int(base[c] * (1 - t) + accents[0][c] * t) for c in range(3)))
    for _ in range(14):
        x, y = rnd.randrange(-80, W + 80), rnd.randrange(-80, H + 80)
        r = rnd.randrange(60, 240)
        col = accents[rnd.randrange(len(accents))]
        d.ellipse([x - r, y - r, x + r, y + r], fill=col)
    img = img.filter(ImageFilter.GaussianBlur(38))
    px = img.load()
    for y in range(0, H, 2):
        for x in range(0, W, 2):
            n = rnd.randrange(-9, 10)
            r, g, b = px[x, y]
            px[x, y] = (max(0, min(255, r + n)), max(0, min(255, g + n)), max(0, min(255, b + n)))
    # Vignette, so the edges are not flat.
    vig = Image.new("L", (W, H), 0)
    vd = ImageDraw.Draw(vig)
    vd.ellipse([-W // 3, -H // 6, W + W // 3, H + H // 6], fill=255)
    img = Image.composite(img, Image.new("RGB", (W, H), (0, 0, 0)),
                          vig.filter(ImageFilter.GaussianBlur(120)))
    save(img.convert("RGBA"), name)


def clearlogo(name):
    img = Image.new("RGBA", (360, 130), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # A mark with curves and a hard edge, so the alpha channel is doing work.
    d.rounded_rectangle([6, 26, 106, 106], radius=20, fill=(250, 250, 250, 255))
    d.rounded_rectangle([22, 44, 90, 88], radius=12, fill=(0, 0, 0, 0))
    for i, (x, w) in enumerate([(126, 46), (182, 46), (238, 46), (294, 52)]):
        d.rounded_rectangle([x, 40 + (i % 2) * 8, x + w, 96 - (i % 2) * 6],
                            radius=8, fill=(250, 250, 250, 255))
    img = img.filter(ImageFilter.GaussianBlur(0.6))
    save(img, name)


def save(img, name):
    img.save(OUT / f"{name}.png")
    (OUT / f"{name}.rgba").write_bytes(img.tobytes())
    print(f"  {name}: {img.size[0]}x{img.size[1]}  png + rgba ({img.size[0]*img.size[1]*4} B)")


print("generating media scene assets...")
backdrop("fanart_a", (18, 26, 48), [(196, 92, 40), (232, 168, 66), (60, 40, 90)], 7)
backdrop("fanart_b", (10, 32, 40), [(38, 132, 148), (96, 196, 180), (24, 48, 96)], 21)
clearlogo("clearlogo")
