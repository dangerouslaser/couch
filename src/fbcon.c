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

static unsigned char *fb;        /* mapped framebuffer */
static unsigned char *shadow;    /* what we draw into */
static int W, H, bpp, stride, fblen, shlen;
static int cols, rows, cur;
static int nbuf, curbuf;         /* hardware buffers available, and the live one */
static int fbfd;
static struct fb_var_screeninfo var;

static unsigned int rgb(int r, int g, int b) { return (r << 16) | (g << 8) | b; }

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
    if (nbuf < 2) {            /* no room to flip: write in place and hope */
        memcpy(fb, shadow, shlen);
        ioctl(fbfd, FBIOPAN_DISPLAY, &var);
        return;
    }
    curbuf = (curbuf + 1) % nbuf;
    memcpy(fb + (long)curbuf * shlen, shadow, shlen);
    var.xoffset = 0;
    var.yoffset = curbuf * H;
    if (ioctl(fbfd, FBIOPAN_DISPLAY, &var) < 0)
        ioctl(fbfd, FBIOPUT_VSCREENINFO, &var);
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
    nbuf  = var.yres_virtual / H;
    if (nbuf < 1) nbuf = 1;
    if (nbuf > 3) nbuf = 3;
    fblen = shlen * nbuf;

    fb = mmap(NULL, fblen, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (fb == MAP_FAILED) { perror("mmap"); return 1; }
    shadow = malloc(shlen);
    if (!shadow) { perror("malloc"); return 1; }

    unsigned int bg = rgb(0x0a, 0x0c, 0x12);
    for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) put_px(x, y, bg);
    curbuf = 0;

    char banner[128];
    snprintf(banner, sizeof banner, "== HA100 Linux %dx%d %dbpp %dx%d %dbuf ==",
             W, H, bpp, cols, rows, nbuf);
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
