#!/bin/sh
# Lucide icons for the Slint UI.
#
# The mockup drew room glyphs as primitives - a square, a circle, a square on
# its point - because the handoff wanted no asset pipeline. Real rooms want
# real icons, and there is already a Lucide pipeline here from the C build.
#
# Rendered at 24px rather than the design's 20: Lucide's 24px viewBox carries
# about 2px of padding, so the visible mark lands at roughly 20px anyway. The
# stroke is widened from Lucide's 2 to 2.2 to sit closer to the 2.5 the design
# asks for without looking heavy at this size.
set -e
cd "$(dirname "$0")/.."
SZ=${ICON_SIZE:-24}
STROKE=${ICON_STROKE:-2.2}
SVG=gui/icons/svg
OUT=ui/couch-gui/assets
mkdir -p "$SVG" "$OUT"

ICONS="sofa bed cooking-pot book-open door-open car trees lamp tv house \
       lightbulb blinds thermometer speaker monitor"

BASE=https://raw.githubusercontent.com/lucide-icons/lucide/main/icons

for i in $ICONS; do
    [ -f "$SVG/$i.svg" ] || curl -sfL -o "$SVG/$i.svg" "$BASE/$i.svg" || {
        echo "  ! $i not found upstream"; rm -f "$SVG/$i.svg"; continue; }
    # Lucide strokes use currentColor; render white on transparent so the alpha
    # channel carries the shape and Slint's colorize can tint it.
    sed -e 's/currentColor/#ffffff/g' \
        -e "s/stroke-width=\"2\"/stroke-width=\"$STROKE\"/" \
        "$SVG/$i.svg" > "$OUT/.$i.svg"
    rsvg-convert -w "$SZ" -h "$SZ" -o "$OUT/icon-$i.png" "$OUT/.$i.svg"
    rm -f "$OUT/.$i.svg"
done
echo "wrote $(ls "$OUT"/icon-*.png 2>/dev/null | wc -l | tr -d ' ') icons at ${SZ}px"
