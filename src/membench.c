/*
 * Is full-screen compositing on this device bandwidth-bound or compute-bound?
 *
 * NEON blenders bought only ~10% on the media scene, which suggests the blend
 * arithmetic is not the cost. This measures the memory traffic underneath it:
 * a plain copy, pure read, pure write, and the read-two-write-one pattern an
 * alpha blend actually performs - the last one both scalar and with NEON, so
 * the two can be compared directly at the same memory traffic.
 *
 * Buffers are a full screen (480*800*4 = 1.5MB), comfortably past any cache on
 * a Cortex-A7, so these are DRAM figures rather than cache figures.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
#include <arm_neon.h>

#define W 480
#define H 800
#define PX (W * H)
#define BYTES ((size_t)PX * 4)
#define ITERS 40

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

static void report(const char *what, double secs, double bytes_moved)
{
    double mb = bytes_moved / (1024.0 * 1024.0);
    printf("  %-34s %7.1f MB/s   %6.2f ms/frame   %5.1f ns/px\n",
           what, mb / secs, secs * 1000.0 / ITERS,
           secs * 1e9 / ((double)ITERS * PX));
}

int main(void)
{
    /* aligned_alloc arrived in bionic after API 21; posix_memalign is there. */
    void *pa, *pb, *pc;
    if (posix_memalign(&pa, 64, BYTES) || posix_memalign(&pb, 64, BYTES) ||
        posix_memalign(&pc, 64, BYTES)) { perror("alloc"); return 1; }
    uint8_t *a = pa, *b = pb, *c = pc;
    memset(a, 0x40, BYTES); memset(b, 0x80, BYTES); memset(c, 0xC0, BYTES);

    printf("membench: %dx%d, %zu KB per buffer, %d iterations\n", W, H, BYTES / 1024, ITERS);

    double t = now_s();
    for (int i = 0; i < ITERS; i++) memcpy(c, a, BYTES);
    report("memcpy (read 1 + write 1)", now_s() - t, (double)ITERS * BYTES * 2);

    volatile uint32_t sink = 0;
    t = now_s();
    for (int i = 0; i < ITERS; i++) {
        const uint32_t *p = (const uint32_t *)a;
        uint32_t s = 0;
        for (size_t j = 0; j < PX; j += 4) s += p[j];
        sink = s;
    }
    report("read only", now_s() - t, (double)ITERS * BYTES);
    (void)sink;

    /* Left in but not to be trusted: the compiler is free to collapse repeated
     * memsets of the same buffer, and the number that comes back (>100 GB/s) is
     * plainly not a DRAM figure. memcpy above is the honest write measurement. */
    t = now_s();
    for (int i = 0; i < ITERS; i++) memset(c, i & 0xff, BYTES);
    report("write only (unreliable, see src)", now_s() - t, (double)ITERS * BYTES);

    /* The blend pattern: read source, read destination, write destination. */
    t = now_s();
    for (int i = 0; i < ITERS; i++) {
        const uint8_t *s = a; uint8_t *d = c;
        for (size_t j = 0; j < PX; j++, s += 4, d += 4) {
            uint32_t m = s[3], inv = 255u - m;
            d[0] = (uint8_t)((s[0] * m + d[0] * inv + 128) >> 8);
            d[1] = (uint8_t)((s[1] * m + d[1] * inv + 128) >> 8);
            d[2] = (uint8_t)((s[2] * m + d[2] * inv + 128) >> 8);
        }
    }
    report("alpha blend, scalar", now_s() - t, (double)ITERS * BYTES * 3);

    t = now_s();
    for (int i = 0; i < ITERS; i++) {
        const uint8_t *s = a; uint8_t *d = c;
        for (size_t j = 0; j + 8 <= PX; j += 8, s += 32, d += 32) {
            uint8x8x4_t sp = vld4_u8(s), dp = vld4_u8(d);
            uint8x8_t m = sp.val[3], inv = vsub_u8(vdup_n_u8(255), m);
            for (int ch = 0; ch < 3; ch++) {
                uint16x8_t x = vaddq_u16(vmull_u8(sp.val[ch], m), vmull_u8(dp.val[ch], inv));
                uint16x8_t y = vaddq_u16(x, vdupq_n_u16(128));
                dp.val[ch] = vshrn_n_u16(vaddq_u16(y, vshrq_n_u16(y, 8)), 8);
            }
            vst4_u8(d, dp);
        }
    }
    report("alpha blend, NEON", now_s() - t, (double)ITERS * BYTES * 3);

    /* Same blend without vld4/vst4. The 4-way structured load/store is the
     * obvious suspect: on Cortex-A7 it is a multi-cycle operation, and the
     * deinterleave is not actually needed - the alpha can be broadcast across
     * its own pixel with a table lookup, and the destination's fourth byte
     * preserved with a select instead of by never touching it. */
    t = now_s();
    {
        static const uint8_t idx_lo_b[8] = { 3, 3, 3, 3, 7, 7, 7, 7 };
        static const uint8_t idx_hi_b[8] = { 11, 11, 11, 11, 15, 15, 15, 15 };
        static const uint8_t keep_b[16] = { 255, 255, 255, 0, 255, 255, 255, 0,
                                            255, 255, 255, 0, 255, 255, 255, 0 };
        const uint8x8_t idx_lo = vld1_u8(idx_lo_b), idx_hi = vld1_u8(idx_hi_b);
        const uint8x16_t keep = vld1q_u8(keep_b);
        const uint8x16_t ones = vdupq_n_u8(255);

        for (int i = 0; i < ITERS; i++) {
            const uint8_t *s = a; uint8_t *d = c;
            for (size_t j = 0; j + 4 <= PX; j += 4, s += 16, d += 16) {
                uint8x16_t sv = vld1q_u8(s), dv = vld1q_u8(d);
                uint8x8x2_t tab; tab.val[0] = vget_low_u8(sv); tab.val[1] = vget_high_u8(sv);
                uint8x16_t m = vcombine_u8(vtbl2_u8(tab, idx_lo), vtbl2_u8(tab, idx_hi));
                uint8x16_t iv = vsubq_u8(ones, m);

                uint16x8_t lo = vmlal_u8(vmull_u8(vget_low_u8(sv), vget_low_u8(m)),
                                         vget_low_u8(dv), vget_low_u8(iv));
                uint16x8_t hi = vmlal_u8(vmull_u8(vget_high_u8(sv), vget_high_u8(m)),
                                         vget_high_u8(dv), vget_high_u8(iv));
                lo = vaddq_u16(lo, vdupq_n_u16(128));
                hi = vaddq_u16(hi, vdupq_n_u16(128));
                uint8x16_t out = vcombine_u8(
                    vshrn_n_u16(vaddq_u16(lo, vshrq_n_u16(lo, 8)), 8),
                    vshrn_n_u16(vaddq_u16(hi, vshrq_n_u16(hi, 8)), 8));
                vst1q_u8(d, vbslq_u8(keep, out, dv));
            }
        }
    }
    report("alpha blend, NEON no vld4", now_s() - t, (double)ITERS * BYTES * 3);

    /* And the one that is not ordinary memory: the panel itself. */
    int fd = open("/dev/fb0", O_RDWR);
    if (fd >= 0) {
        void *fb = mmap(NULL, BYTES, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        if (fb != MAP_FAILED) {
            t = now_s();
            for (int i = 0; i < ITERS; i++) memcpy(fb, a, BYTES);
            report("memcpy RAM -> framebuffer", now_s() - t, (double)ITERS * BYTES * 2);
            munmap(fb, BYTES);
        }
        close(fd);
    }

    printf("\n  the media scene moves ~13 MB per crossfade frame across 3 passes\n");
    return 0;
}
