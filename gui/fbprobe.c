/*
 * One question, slowly: does an mmap write reach the panel with no ioctl?
 *
 * Establish a known state with a pan, then paint via mmap only and hold for a
 * long time. If the colour appears during the mmap-only window, no ioctl is
 * needed per frame and a repaint costs a memcpy rather than a 17ms vsync wait.
 */
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/ioctl.h>
#include <linux/fb.h>

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    int fd = open("/dev/fb0", O_RDWR);
    struct fb_var_screeninfo var;
    struct fb_fix_screeninfo fix;
    ioctl(fd, FBIOGET_VSCREENINFO, &var);
    ioctl(fd, FBIOGET_FSCREENINFO, &fix);

    size_t page = (size_t)fix.line_length * var.yres;
    uint8_t *map = mmap(NULL, page * (var.yres_virtual / var.yres),
                        PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    uint32_t *px = (uint32_t *)map;
    size_t npx = page / 4;

    int bl = open("/sys/class/leds/lcd-backlight/brightness", O_WRONLY);
    if (bl >= 0) { write(bl, "255\n", 4); close(bl); }

    /* Known state: solid BLACK, published with a pan so we are certain. */
    for (size_t i = 0; i < npx; i++) px[i] = 0xFF000000;
    var.xoffset = var.yoffset = 0;
    ioctl(fd, FBIOPAN_DISPLAY, &var);
    printf(">>> screen should now be BLACK. holding 8s.\n");
    sleep(8);

    /* The actual test: paint white via mmap ONLY. No ioctl follows. */
    printf(">>> writing WHITE via mmap only - NO ioctl. holding 15s.\n");
    printf(">>> if the screen turns white during this window, mmap alone works.\n");
    for (size_t i = 0; i < npx; i++) px[i] = 0xFFFFFFFF;
    sleep(15);

    printf(">>> now issuing the pan.\n");
    ioctl(fd, FBIOPAN_DISPLAY, &var);
    printf(">>> if it only turned white JUST NOW, the ioctl is required.\n");
    sleep(6);
    return 0;
}
