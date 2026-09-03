/* The fanart and logo, linked in directly. As C arrays these would be ~9MB of
 * source text per backdrop and minutes of compile time; incbin is instant and
 * the bytes land in .rodata either way.
 *
 * Stored plain R,G,B,A: LVGL blits image bytes through unchanged, so they
 * reach the panel in the order it wants. couch_rgb() is for colours only. */
    .section .rodata
    .balign 4
    .global couch_fanart_a
couch_fanart_a:
    .incbin "../assets/fanart_a.rgba"
    .balign 4
    .global couch_fanart_b
couch_fanart_b:
    .incbin "../assets/fanart_b.rgba"
    .balign 4
    .global couch_clearlogo
couch_clearlogo:
    .incbin "../assets/clearlogo.rgba"
