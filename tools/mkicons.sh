#!/bin/sh
# Build LVGL icon assets from Lucide SVGs.
#
# lucide-react cannot run here, but the artwork is just SVG. Rasterise each icon
# and convert to LVGL's A8 format: alpha only, so a single asset can be tinted
# any colour at runtime, which is how Lucide's currentColor behaves on the web.
set -e
cd "$(dirname "$0")/.."
SZ=${ICON_SIZE:-28}
SVG=gui/icons/svg
PNG=gui/icons/png
OUT=gui/icons
mkdir -p "$SVG" "$PNG" "$OUT"

ICONS="tv lightbulb blinds power volume-2 volume-x play pause skip-forward \
       skip-back square settings wifi house chevron-right chevron-left \
       thermometer sun moon speaker cast monitor film lock"

BASE=https://raw.githubusercontent.com/lucide-icons/lucide/main/icons

for i in $ICONS; do
    [ -f "$SVG/$i.svg" ] || curl -sfL -o "$SVG/$i.svg" "$BASE/$i.svg" || {
        echo "  ! $i not found upstream"; rm -f "$SVG/$i.svg"; continue; }
    # Lucide strokes use currentColor; render white on transparent so the alpha
    # channel carries the shape and LVGL can recolour it.
    sed 's/currentColor/#ffffff/g' "$SVG/$i.svg" > "$PNG/$i.svg"
    rsvg-convert -w "$SZ" -h "$SZ" -o "$PNG/$i.png" "$PNG/$i.svg"
    rm -f "$PNG/$i.svg"
done
echo "rasterised $(ls "$PNG"/*.png 2>/dev/null | wc -l | tr -d ' ') icons at ${SZ}px"

# LVGL's own LVGLImage.py needs pypng; Pillow is already present, so convert
# directly rather than adding a dependency.
python3 tools/png2lvgl.py gui/icons.c gui/icons.h "$PNG"/*.png
