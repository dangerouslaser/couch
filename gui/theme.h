/*
 * shadcn/ui's design language, expressed as LVGL styles.
 *
 * shadcn is React + Radix + Tailwind and cannot run here, but its design is a
 * small set of tokens plus consistent radii, spacing and focus treatment. Those
 * port cleanly. Values are the default "zinc" dark theme, converted from the
 * HSL custom properties shadcn ships.
 *
 * Sizes are scaled up from the web defaults: this is a 3.1" 480x800 panel at
 * ~220dpi driven mostly by hardware buttons, so touch targets and type are
 * larger than a desktop would use.
 */
#pragma once
#include "lvgl/lvgl.h"

/* --- tokens (shadcn zinc, dark) ------------------------------------------ */
#define C_BACKGROUND        0x09090b   /* 240 10%  3.9% */
#define C_FOREGROUND        0xfafafa   /*   0  0%   98% */
#define C_CARD              0x09090b
#define C_MUTED             0x27272a   /* 240 3.7% 15.9% */
#define C_MUTED_FOREGROUND  0xa1a1aa   /* 240   5% 64.9% */
#define C_ACCENT            0x27272a
#define C_ACCENT_FOREGROUND 0xfafafa
#define C_PRIMARY           0xfafafa
#define C_PRIMARY_FG        0x18181b
#define C_SECONDARY         0x27272a
#define C_BORDER            0x27272a
#define C_RING              0xd4d4d8   /* 240 4.9% 83.9% */
#define C_DESTRUCTIVE       0xdc2626
#define C_SUCCESS           0x22c55e

/* --- geometry: shadcn's --radius is 0.5rem; scaled for this panel --------- */
#define R_LG   12
#define R_MD    9
#define R_SM    6
#define PAD_PAGE   16
#define ROW_H      60
#define GAP         8

typedef enum {
    BTN_DEFAULT,     /* bg-primary  text-primary-foreground */
    BTN_SECONDARY,   /* bg-secondary                        */
    BTN_OUTLINE,     /* border-input, transparent bg        */
    BTN_GHOST,       /* transparent until focused           */
    BTN_DESTRUCTIVE,
} couch_btn_variant_t;

/* The panel reads the low byte as red (fb_var_screeninfo: red=0/8) while LVGL
 * lays out a 32-bit pixel as B,G,R,X in memory. Rather than walk every pixel of
 * every frame swapping them - which cost more than the memcpy that followed it,
 * and was invisible in the flush timing because it happened before the clock
 * started - swap the two channels once, here, when a colour is created.
 * Blending is per-channel and linear, so consistently swapped inputs produce
 * correctly swapped output.
 *
 * Use this instead of lv_color_hex() everywhere in this app. */
static inline lv_color_t couch_rgb(uint32_t hex)
{
    return lv_color_make((uint8_t)(hex & 0xff),           /* blue  -> red slot  */
                         (uint8_t)((hex >> 8) & 0xff),
                         (uint8_t)((hex >> 16) & 0xff));  /* red   -> blue slot */
}

void      couch_theme_init(void);
lv_obj_t *couch_screen(void);
lv_obj_t *couch_card(lv_obj_t *parent);
lv_obj_t *couch_group(lv_obj_t *parent);   /* card without the border */
lv_obj_t *couch_h1(lv_obj_t *parent, const char *text);
lv_obj_t *couch_muted(lv_obj_t *parent, const char *text);
lv_obj_t *couch_separator(lv_obj_t *parent);
lv_obj_t *couch_button(lv_obj_t *parent, couch_btn_variant_t v, const char *text);
lv_obj_t *couch_row(lv_obj_t *parent, const lv_image_dsc_t *icon,
                    const char *title, const char *value);
