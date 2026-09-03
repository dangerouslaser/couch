/*
 * fbcon - a console on the framebuffer, for a kernel built without CONFIG_VT.
 *
 * This kernel has no virtual terminal, so there is no fbcon, no /dev/tty0 and no
 * boot messages on screen: a working boot looks exactly like a hung one. This
 * reads lines on stdin and draws them to /dev/fb0 so init can narrate itself.
 *
 * Scrolls by moving the framebuffer up one text row and drawing only the new
 * line, rather than repainting 1.5MB per line.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/ioctl.h>
#include <linux/fb.h>
#include "font.h"

#define CW 8
#define CH 16

static unsigned char *shadow;    /* what we draw into */
static int W, H, bpp, stride, fblen, shlen;
static int cols, rows, cur;
static int fbfd;

/* mtkfb composites ARGB8888: a pixel with alpha 0 is fully transparent, so it
 * reads back correctly from /dev/fb0 and is never visible on the panel.
 *
 * The channel order is ABGR: this panel reads the low byte as red, exactly as
 * fb_var_screeninfo reports (red=0/8, green=8/8, blue=16/8).
 *
 * An earlier version had this backwards. It was "confirmed" against a pale blue
 * (#7ab8ff), where a red/blue swap is genuinely ambiguous by eye; a labelled
 * test pattern of saturated primaries settled it in seconds. fbcon looked fine
 * either way because its output is almost all light grey text, where swapping
 * red and blue barely shifts the shade. */
static unsigned int rgb(int r, int g, int b)
{ return 0xFF000000u | (b << 16) | (g << 8) | r; }

static void put_px(int x, int y, unsigned int c)
{
    if (x < 0 || y < 0 || x >= W || y >= H) return;
    unsigned char *p = shadow + (long)y * stride + (long)x * (bpp / 8);
    if (bpp == 32) *(unsigned int *)p = c;
    else if (bpp == 16)
        *(unsigned short *)p = ((c >> 8) & 0xf800) | ((c >> 5) & 0x07e0) | ((c >> 3) & 0x001f);
}

static void draw_char(int col, int row, unsigned char ch, unsigned int fg, unsigned int bg)
{
    const unsigned char *g = FONT + (int)ch * CH;
    for (int y = 0; y < CH; y++) {
        unsigned char bits = g[y];
        for (int x = 0; x < CW; x++)
            put_px(col * CW + x, row * CH + y, (bits & (0x80 >> x)) ? fg : bg);
    }
}

static void clear_row(int row, unsigned int bg)
{
    for (int y = 0; y < CH; y++)
        for (int x = 0; x < W; x++)
            put_px(x, row * CH + y, bg);
}

static void scroll(unsigned int bg)
{
    memmove(shadow, shadow + (long)CH * stride, (long)(rows - 1) * CH * stride);
    clear_row(rows - 1, bg);
}

/* Publish the shadow: copy into the next buffer and pan to it. A command-mode
   panel ignores writes to the buffer it is already showing. */
static void flip(void)
{
    /* Publish with write(), not mmap.
     *
     * mtkfb drives a MIPI command-mode panel: pixels reach the glass only when
     * the driver pushes a frame. Writes through an mmap never trigger that, so
     * the buffer ends up holding a perfectly correct image that is never shown -
     * readable back from /dev/fb0, invisible on the panel. write() does trigger
     * it, which is why plain dd displayed fine all along. */
    /* Push in 4096-byte chunks. mtkfb transfers to the panel per write(), so a
     * single 1.5MB write lands in memory and never reaches the glass - which is
     * why plain dd displayed and one big write did not. */
    lseek(fbfd, 0, SEEK_SET);
    ssize_t left = shlen;
    const unsigned char *p = shadow;
    while (left > 0) {
        ssize_t want = left < 4096 ? left : 4096;
        ssize_t n = write(fbfd, p, want);
        if (n <= 0) break;
        p += n; left -= n;
    }
    /* This panel switches its own backlight off when idle, and a dark screen is
     * indistinguishable from a crashed one. */
    int b = open("/sys/class/leds/lcd-backlight/brightness", O_WRONLY);
    if (b >= 0) { write(b, "255\n", 4); close(b); }
}

static void draw_line(const char *s)
{
    /* A leading marker tints the line so failures stand out at a glance. */
    unsigned int fg = rgb(0xd0, 0xd8, 0xe0), bg = rgb(0x0a, 0x0c, 0x12);
    if (strstr(s, "FAIL") || strstr(s, "ERROR") || strstr(s, "MISSING"))
        fg = rgb(0xff, 0x6b, 0x6b);
    else if (strstr(s, "ok") || strstr(s, "OK") || strstr(s, "up"))
        fg = rgb(0x7c, 0xe0, 0x9b);
    else if (s[0] == '=' || s[0] == '[')
        fg = rgb(0x7a, 0xb8, 0xff);

    if (cur >= rows) { scroll(bg); cur = rows - 1; }
    clear_row(cur, bg);
    for (int i = 0; i < cols && s[i]; i++)
        draw_char(i, cur, (unsigned char)s[i], fg, bg);
    cur++;
}

int main(int argc, char **argv)
{
    const char *dev = argc > 1 ? argv[1] : "/dev/fb0";
    int fd = open(dev, O_RDWR);
    if (fd < 0) { perror("open fb"); return 1; }

    struct fb_var_screeninfo var;
    struct fb_fix_screeninfo fix;
    if (ioctl(fd, FBIOGET_VSCREENINFO, &var) < 0) { perror("VSCREENINFO"); return 1; }
    if (ioctl(fd, FBIOGET_FSCREENINFO, &fix) < 0) { perror("FSCREENINFO"); return 1; }

    W = var.xres; H = var.yres; bpp = var.bits_per_pixel;
    stride = fix.line_length ? fix.line_length : W * (bpp / 8);
    fblen = stride * H;
    cols = W / CW; rows = H / CH;

    fbfd = fd;
    shlen = stride * H;
    shadow = malloc(shlen);
    if (!shadow) { perror("malloc"); return 1; }

    unsigned int bg = rgb(0x0a, 0x0c, 0x12);
    for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) put_px(x, y, bg);

    FILE *g = fopen("/tmp/fbcon.geom", "w");
    if (g) {
        fprintf(g, "xres=%u yres=%u xres_virtual=%u yres_virtual=%u\n",
                var.xres, var.yres, var.xres_virtual, var.yres_virtual);
        fprintf(g, "bpp=%u line_length=%u smem_len=%u\n",
                var.bits_per_pixel, fix.line_length, fix.smem_len);
        fprintf(g, "red=%u/%u green=%u/%u blue=%u/%u transp=%u/%u\n",
                var.red.offset, var.red.length, var.green.offset, var.green.length,
                var.blue.offset, var.blue.length, var.transp.offset, var.transp.length);
        fprintf(g, "cols=%d rows=%d stride_used=%d\n", cols, rows, stride);
        fclose(g);
    }

    char banner[128];
    snprintf(banner, sizeof banner, "== HA100 Linux %dx%d %dbpp %dx%d ==",
             W, H, bpp, cols, rows);
    draw_line(banner);
    flip();

    char line[512];
    while (fgets(line, sizeof line, stdin)) {
        line[strcspn(line, "\r\n")] = 0;
        draw_line(line);
        flip();
    }
    return 0;
}
