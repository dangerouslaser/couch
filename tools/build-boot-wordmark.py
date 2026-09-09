#!/usr/bin/env python3
"""Render Couch's wordmark and replace only the boot image in an HA100 logo dump.

Requires Pillow, fonttools and brotli. Input must be a backup of the target's
logo partition. This tool never flashes a device; inspect the PNG before use.
"""
import argparse
import io
from pathlib import Path
import struct
import zlib

from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from PIL import Image, ImageDraw, ImageFont

import mtklogo


def wordmark():
    root = Path(__file__).resolve().parent.parent
    font = TTFont(root / 'assets/inter/InterVariable.woff2')
    font = instantiateVariableFont(font, {'wght': 800, 'opsz': 32}, inplace=True)
    font.flavor = None
    data = io.BytesIO()
    font.save(data)
    data.seek(0)
    # Supersampling preserves the Inter outlines at the panel's native size.
    scale, size, spacing = 4, 88, -2
    face = ImageFont.truetype(data, size * scale)
    text = 'couch.'
    layer = Image.new('RGBA', (480 * scale, 160 * scale))
    draw = ImageDraw.Draw(layer)
    x = 0
    for index, character in enumerate(text):
        draw.text((x, 0), character, font=face, fill='white')
        # Pair lengths retain kerning while applying precisely -2px tracking.
        advance = face.getlength(character)
        if index + 1 < len(text):
            advance = face.getlength(text[index:index + 2]) - face.getlength(text[index + 1])
        x += advance + spacing * scale
    layer = layer.crop(layer.getbbox())
    layer = layer.resize((round(layer.width / scale), round(layer.height / scale)), Image.Resampling.LANCZOS)
    frame = Image.new('RGBA', (480, 800), 'black')
    frame.alpha_composite(layer, ((480 - layer.width) // 2, (800 - layer.height) // 2))
    return frame


def build(source, destination, preview):
    original = source.read_bytes()
    name, blobs = mtklogo.unpack(source)
    if name != 'logo' or len(zlib.decompress(blobs[0])) != 480 * 800 * 4:
        raise ValueError('expected HA100 logo partition with a 480x800 BGRA boot frame')
    frame = wordmark()
    frame.save(preview)
    replacement = zlib.compress(frame.tobytes('raw', 'BGRA'), 9)
    packed = bytearray(mtklogo.pack(name, [replacement, *blobs[1:]]))
    if len(packed) > len(original):
        raise ValueError('replacement exceeds partition size')
    # Preserve the target header (including vendor extension fields), changing
    # only payload length. Retain partition length and all other image streams.
    packed[:512] = original[:512]
    struct.pack_into('<I', packed, 4, len(packed) - 512)
    packed.extend(b'\0' * (len(original) - len(packed)))
    destination.write_bytes(packed)
    _, verified = mtklogo.unpack(destination)
    assert verified[1:] == blobs[1:]
    assert zlib.decompress(verified[0]) == frame.tobytes('raw', 'BGRA')
    print(f'{destination}: {len(packed)} bytes; only boot frame 0 replaced; preview: {preview}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--preview', type=Path, default=Path('build/couch-boot.png'))
    args = parser.parse_args()
    if args.source.resolve() == args.destination.resolve():
        parser.error('keep the original backup: source and destination must differ')
    build(args.source, args.destination, args.preview)
