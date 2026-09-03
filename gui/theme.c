#include "theme.h"

/* shadcn's focus ring: drawn outside the object with a gap of page background
 * between, which is what makes it read against any fill - including the
 * near-white primary button, where an inset 2px border is the only difference
 * between focused and not and is barely visible.
 *
 * An outline is painted outside the bounds and LVGL clips children to their
 * parent, so every container holding a focusable object needs padding of at
 * least (ring width + pad). couch_card already has it; see the actions row in
 * build_ui. */
#define RING_W   3
#define RING_PAD 3

static void apply_focus_ring(lv_obj_t *o, uint32_t ring)
{
    lv_obj_set_style_outline_color(o, lv_color_hex(ring), LV_STATE_FOCUSED);
    lv_obj_set_style_outline_width(o, RING_W, LV_STATE_FOCUSED);
    lv_obj_set_style_outline_pad(o, RING_PAD, LV_STATE_FOCUSED);
    lv_obj_set_style_outline_opa(o, LV_OPA_COVER, LV_STATE_FOCUSED);
    /* No border on focus: that was the inset fallback and it double-draws. */
    lv_obj_set_style_border_width(o, 0, LV_STATE_FOCUSED);
}

void couch_theme_init(void)
{
    /* Replace LVGL's default theme rather than only restyling our widgets.
     * Its primary colour is blue (#2196F3), which keeps surfacing in states and
     * transitions we have not explicitly overridden - and the focus fade
     * interpolates through it, so it shows up as a blue flash on navigation. */
    lv_display_t *disp = lv_display_get_default();
    lv_theme_t *th = lv_theme_default_init(disp,
                                           lv_color_hex(C_PRIMARY),
                                           lv_color_hex(C_MUTED),
                                           true,                 /* dark */
                                           LV_FONT_DEFAULT);
    lv_display_set_theme(disp, th);

    lv_obj_t *scr = lv_screen_active();
    lv_obj_set_style_bg_color(scr, lv_color_hex(C_BACKGROUND), 0);
    lv_obj_set_style_text_color(scr, lv_color_hex(C_FOREGROUND), 0);
    lv_obj_set_style_pad_all(scr, PAD_PAGE, 0);
}

lv_obj_t *couch_card(lv_obj_t *parent)
{
    lv_obj_t *c = lv_obj_create(parent);
    lv_obj_set_style_bg_color(c, lv_color_hex(C_CARD), 0);
    lv_obj_set_style_border_color(c, lv_color_hex(C_BORDER), 0);
    lv_obj_set_style_border_width(c, 1, 0);
    lv_obj_set_style_radius(c, R_LG, 0);
    lv_obj_set_style_pad_all(c, GAP, 0);
    lv_obj_set_style_pad_row(c, GAP, 0);
    lv_obj_set_flex_flow(c, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_scrollbar_mode(c, LV_SCROLLBAR_MODE_OFF);
    return c;
}

/* A card without the visible outline: the rows carry their own focus treatment,
 * so on a small dark panel the border mostly adds noise. The padding stays -
 * dropping it too pulls the rows flush against the page edge and the focus ring
 * loses the breathing room the border used to provide. */
lv_obj_t *couch_group(lv_obj_t *parent)
{
    lv_obj_t *c = couch_card(parent);
    lv_obj_set_style_border_width(c, 0, 0);
    lv_obj_set_style_bg_opa(c, LV_OPA_TRANSP, 0);
    return c;
}

lv_obj_t *couch_h1(lv_obj_t *parent, const char *text)
{
    lv_obj_t *l = lv_label_create(parent);
    lv_label_set_text(l, text);
    lv_obj_set_style_text_color(l, lv_color_hex(C_FOREGROUND), 0);
    lv_obj_set_style_text_font(l, &lv_font_montserrat_28, 0);
    return l;
}

lv_obj_t *couch_muted(lv_obj_t *parent, const char *text)
{
    lv_obj_t *l = lv_label_create(parent);
    lv_label_set_text(l, text);
    lv_obj_set_style_text_color(l, lv_color_hex(C_MUTED_FOREGROUND), 0);
    lv_obj_set_style_text_font(l, &lv_font_montserrat_14, 0);
    return l;
}

lv_obj_t *couch_separator(lv_obj_t *parent)
{
    lv_obj_t *s = lv_obj_create(parent);
    lv_obj_remove_style_all(s);
    lv_obj_set_size(s, LV_PCT(100), 1);
    lv_obj_set_style_bg_color(s, lv_color_hex(C_BORDER), 0);
    lv_obj_set_style_bg_opa(s, LV_OPA_COVER, 0);
    return s;
}

lv_obj_t *couch_button(lv_obj_t *parent, couch_btn_variant_t v, const char *text)
{
    lv_obj_t *b = lv_button_create(parent);
    lv_obj_set_height(b, 52);
    lv_obj_set_style_radius(b, R_MD, 0);
    lv_obj_set_style_border_width(b, 0, 0);
    lv_obj_set_style_shadow_width(b, 0, 0);
    lv_obj_set_style_pad_hor(b, 16, 0);

    uint32_t bg = C_SECONDARY, fg = C_FOREGROUND;
    switch (v) {
    case BTN_DEFAULT:     bg = C_PRIMARY;     fg = C_PRIMARY_FG; break;
    case BTN_SECONDARY:   bg = C_SECONDARY;   fg = C_FOREGROUND; break;
    case BTN_DESTRUCTIVE: bg = C_DESTRUCTIVE; fg = C_FOREGROUND; break;
    case BTN_OUTLINE:
        lv_obj_set_style_bg_opa(b, LV_OPA_TRANSP, 0);
        lv_obj_set_style_border_width(b, 1, 0);
        lv_obj_set_style_border_color(b, lv_color_hex(C_BORDER), 0);
        lv_obj_set_style_border_width(b, 1, LV_STATE_FOCUSED);
        bg = C_BACKGROUND;
        break;
    case BTN_GHOST:
        lv_obj_set_style_bg_opa(b, LV_OPA_TRANSP, 0);
        break;
    }
    if (v != BTN_GHOST && v != BTN_OUTLINE)
        lv_obj_set_style_bg_color(b, lv_color_hex(bg), 0);

    /* Focus adds a ring and leaves the fill alone, as shadcn does. Overriding
     * the background for every variant put dark accent behind the primary
     * button's dark label, which is unreadable. Only the transparent variants
     * take a background on focus - the equivalent of shadcn's hover. */
    if (v == BTN_GHOST || v == BTN_OUTLINE) {
        lv_obj_set_style_bg_color(b, lv_color_hex(C_ACCENT), LV_STATE_FOCUSED);
        lv_obj_set_style_bg_opa(b, LV_OPA_COVER, LV_STATE_FOCUSED);
    } else {
        /* Solid variants keep their fill; pressed dims slightly for feedback. */
        lv_obj_set_style_bg_color(b, lv_color_hex(bg), LV_STATE_FOCUSED);
        lv_obj_set_style_bg_opa(b, LV_OPA_COVER, LV_STATE_FOCUSED);
        lv_obj_set_style_bg_opa(b, LV_OPA_80, LV_STATE_PRESSED);
    }
    apply_focus_ring(b, C_RING);

    if (text) {
        lv_obj_t *l = lv_label_create(b);
        lv_label_set_text(l, text);
        lv_obj_set_style_text_color(l, lv_color_hex(fg), 0);
        lv_obj_set_style_text_color(l, lv_color_hex(fg), LV_STATE_FOCUSED);
        lv_obj_set_style_text_font(l, &lv_font_montserrat_20, 0);
        lv_obj_center(l);
    }
    return b;
}

lv_obj_t *couch_row(lv_obj_t *parent, const lv_image_dsc_t *icon,
                    const char *title, const char *value)
{
    lv_obj_t *row = lv_button_create(parent);
    lv_obj_set_size(row, LV_PCT(100), ROW_H);
    lv_obj_set_style_radius(row, R_MD, 0);
    lv_obj_set_style_bg_opa(row, LV_OPA_TRANSP, 0);
    lv_obj_set_style_border_width(row, 0, 0);
    lv_obj_set_style_shadow_width(row, 0, 0);
    lv_obj_set_style_pad_hor(row, 12, 0);
    lv_obj_set_style_bg_color(row, lv_color_hex(C_ACCENT), LV_STATE_FOCUSED);
    lv_obj_set_style_bg_opa(row, LV_OPA_COVER, LV_STATE_FOCUSED);
    apply_focus_ring(row, C_RING);

    if (icon) {
        lv_obj_t *im = lv_image_create(row);
        lv_image_set_src(im, icon);
        /* A8 assets carry only coverage, so recolour like currentColor. */
        lv_obj_set_style_image_recolor(im, lv_color_hex(C_FOREGROUND), 0);
        lv_obj_set_style_image_recolor_opa(im, LV_OPA_COVER, 0);
        lv_obj_align(im, LV_ALIGN_LEFT_MID, 0, 0);
    }

    lv_obj_t *t = lv_label_create(row);
    lv_label_set_text(t, title);
    lv_obj_set_style_text_color(t, lv_color_hex(C_FOREGROUND), 0);
    lv_obj_set_style_text_font(t, &lv_font_montserrat_20, 0);
    lv_obj_align(t, LV_ALIGN_LEFT_MID, icon ? 42 : 0, 0);

    if (value) {
        lv_obj_t *v = lv_label_create(row);
        lv_label_set_text(v, value);
        lv_obj_set_style_text_color(v, lv_color_hex(C_MUTED_FOREGROUND), 0);
        lv_obj_set_style_text_font(v, &lv_font_montserrat_14, 0);
        lv_obj_align(v, LV_ALIGN_RIGHT_MID, 0, 0);
    }
    return row;
}
