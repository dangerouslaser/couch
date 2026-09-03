/*
 * NEON blenders for the paths this app actually spends its time in.
 *
 * LVGL ships its own NEON implementation, but lv_blend_neon.S is written in
 * GNU as macro syntax (.altmacro with & concatenation) that LLVM's integrated
 * assembler rejects, and the NDK is LLVM-only, so there is no gas to fall back
 * to. LVGL's LV_DRAW_SW_ASM_CUSTOM hook lets us supply intrinsics instead.
 *
 * Only the cases that dominate the media scene are implemented; everything
 * else returns LV_RESULT_INVALID and LVGL uses its C loop. The destination is
 * always 4 bytes per pixel here (XRGB8888), and the fourth byte is preserved:
 * mtkfb composites, so clearing alpha would make the frame invisible.
 */
#pragma once

#include <arm_neon.h>
/* This header is pulled into translation units that have only a forward
 * declaration of the blend descriptors (lv_draw_sw.c, lv_draw_sw_img.c), so
 * bring in the definitions rather than relying on the include site. Resolved
 * via the -I on the lvgl checkout; the header has its own guard. */
#include "src/draw/sw/blend/lv_draw_sw_blend_private.h"

/* x/255, rounded, without a divide: the standard (x + 128 + ((x + 128) >> 8)) >> 8. */
static inline uint8x8_t couch_div255_u16(uint16x8_t x)
{
    uint16x8_t t = vaddq_u16(x, vdupq_n_u16(128));
    return vshrn_n_u16(vaddq_u16(t, vshrq_n_u16(t, 8)), 8);
}

static inline uint8_t couch_div255_scalar(uint32_t x)
{
    x += 128;
    return (uint8_t)((x + (x >> 8)) >> 8);
}

/* dst = src over dst, at coverage `mix`. Three channels; the fourth is left. */
static inline void couch_mix24_scalar(const uint8_t *s, uint8_t *d, uint32_t mix)
{
    uint32_t inv = 255u - mix;
    d[0] = couch_div255_scalar(s[0] * mix + d[0] * inv);
    d[1] = couch_div255_scalar(s[1] * mix + d[1] * inv);
    d[2] = couch_div255_scalar(s[2] * mix + d[2] * inv);
}

static inline lv_result_t couch_neon_argb_over_x8888(lv_draw_sw_blend_image_dsc_t *dsc,
                                                     uint32_t dst_px_size, uint32_t opa)
{
    if (dst_px_size != 4 || dsc->mask_buf != NULL) return LV_RESULT_INVALID;

    uint8_t *dst_row = (uint8_t *)dsc->dest_buf;
    const uint8_t *src_row = (const uint8_t *)dsc->src_buf;
    const int32_t w = dsc->dest_w, h = dsc->dest_h;
    const uint8x8_t opa_v = vdup_n_u8((uint8_t)opa);
    const uint8x8_t full = vdup_n_u8(255);

    for (int32_t y = 0; y < h; y++) {
        const uint8_t *s = src_row;
        uint8_t *d = dst_row;
        int32_t x = 0;

        for (; x + 8 <= w; x += 8, s += 32, d += 32) {
            uint8x8x4_t sp = vld4_u8(s);      /* B G R A, deinterleaved */
            uint8x8x4_t dp = vld4_u8(d);

            uint8x8_t mix = (opa == 255) ? sp.val[3]
                                         : couch_div255_u16(vmull_u8(sp.val[3], opa_v));
            uint8x8_t inv = vsub_u8(full, mix);

            dp.val[0] = couch_div255_u16(vaddq_u16(vmull_u8(sp.val[0], mix),
                                                   vmull_u8(dp.val[0], inv)));
            dp.val[1] = couch_div255_u16(vaddq_u16(vmull_u8(sp.val[1], mix),
                                                   vmull_u8(dp.val[1], inv)));
            dp.val[2] = couch_div255_u16(vaddq_u16(vmull_u8(sp.val[2], mix),
                                                   vmull_u8(dp.val[2], inv)));
            /* dp.val[3] is whatever the destination already held. */
            vst4_u8(d, dp);
        }
        for (; x < w; x++, s += 4, d += 4) {
            uint32_t mix = (opa == 255) ? s[3] : couch_div255_scalar(s[3] * opa);
            couch_mix24_scalar(s, d, mix);
        }
        dst_row += dsc->dest_stride;
        src_row += dsc->src_stride;
    }
    return LV_RESULT_OK;
}

static inline lv_result_t couch_neon_color_over_x8888(lv_draw_sw_blend_fill_dsc_t *dsc,
                                                      uint32_t dst_px_size)
{
    if (dst_px_size != 4 || dsc->mask_buf != NULL) return LV_RESULT_INVALID;

    uint8_t *dst_row = (uint8_t *)dsc->dest_buf;
    const int32_t w = dsc->dest_w, h = dsc->dest_h;
    const uint32_t opa = dsc->opa;
    const uint8_t src[3] = { dsc->color.blue, dsc->color.green, dsc->color.red };
    const uint8x8_t mix = vdup_n_u8((uint8_t)opa);
    const uint8x8_t inv = vdup_n_u8((uint8_t)(255u - opa));
    const uint8x8_t sb = vdup_n_u8(src[0]), sg = vdup_n_u8(src[1]), sr = vdup_n_u8(src[2]);

    for (int32_t y = 0; y < h; y++) {
        uint8_t *d = dst_row;
        int32_t x = 0;
        for (; x + 8 <= w; x += 8, d += 32) {
            uint8x8x4_t dp = vld4_u8(d);
            dp.val[0] = couch_div255_u16(vaddq_u16(vmull_u8(sb, mix), vmull_u8(dp.val[0], inv)));
            dp.val[1] = couch_div255_u16(vaddq_u16(vmull_u8(sg, mix), vmull_u8(dp.val[1], inv)));
            dp.val[2] = couch_div255_u16(vaddq_u16(vmull_u8(sr, mix), vmull_u8(dp.val[2], inv)));
            vst4_u8(d, dp);
        }
        for (; x < w; x++, d += 4) couch_mix24_scalar(src, d, opa);
        dst_row += dsc->dest_stride;
    }
    return LV_RESULT_OK;
}

#define LV_DRAW_SW_ARGB8888_BLEND_NORMAL_TO_RGB888(dsc, dst_px_size) \
    couch_neon_argb_over_x8888((dsc), (dst_px_size), 255)

#define LV_DRAW_SW_ARGB8888_BLEND_NORMAL_TO_RGB888_WITH_OPA(dsc, dst_px_size) \
    couch_neon_argb_over_x8888((dsc), (dst_px_size), (dsc)->opa)

#define LV_DRAW_SW_COLOR_BLEND_TO_RGB888_WITH_OPA(dsc, dst_px_size) \
    couch_neon_color_over_x8888((dsc), (dst_px_size))
