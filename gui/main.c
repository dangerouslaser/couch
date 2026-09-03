/*
 * Couch GUI - an LVGL shell for the Sanytron HA100.
 *
 * No X and no Wayland here: this kernel has no CONFIG_VT (so Xorg cannot open a
 * console) and mtkfb exposes no DRM/KMS (so wlroots has no backend). LVGL talks
 * to /dev/fb0 and evdev directly, which needs neither.
 *
 * Two panel quirks, both learned the hard way with fbcon:
 *   - mtkfb transfers a frame to the glass only on write(). Writes through an
 *     mmap land in memory and are never displayed, so LVGL's own fbdev driver
 *     (which mmaps) shows nothing. We supply our own flush instead of patching
 *     the library.
 *   - The panel composites ARGB8888. A pixel with alpha 0 is fully transparent,
 *     so LVGL's rendered buffer has to have its alpha byte forced opaque.
 *
 * Input mirrors the device: a touchscreen plus a hardware keypad. LVGL's input
 * groups give the keypad real focus traversal, which is the interaction model
 * the remote already uses - touch optional, keys sufficient.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <time.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/time.h>
#include <linux/fb.h>
#include <linux/input.h>
#include <errno.h>
#include "lvgl/lvgl.h"
#include "lvgl/src/drivers/evdev/lv_evdev.h"

#define TOUCH_DEV  "/dev/input/event3"   /* mtk-tpd     */
#define KEYPAD_DEV "/dev/input/event1"   /* mt_gpio_kpd */
#define BACKLIGHT  "/sys/class/leds/lcd-backlight/brightness"

static int      fbfd = -1;
static uint32_t scr_w, scr_h;
static size_t   fb_bytes;

static uint32_t millis(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint32_t)(ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
}

static void backlight_on(void)
{
    int fd = open(BACKLIGHT, O_WRONLY);
    if (fd >= 0) { write(fd, "255\n", 4); close(fd); }
}

/* Publish only the rows that changed.
 *
 * A full frame is 1.5MB and this panel accepts about 14MB/s, so pushing the
 * whole screen costs ~105ms and caps the UI near 9fps no matter how fast we
 * draw. Framebuffer rows are contiguous, so writing whole rows covering the
 * dirty area is both simple and dramatically less data: touching one list row
 * is a few percent of the screen.
 *
 * LVGL may call this several times per refresh, so accumulate the dirty row
 * span and push once when the last area arrives. */
static uint64_t flush_us, flush_n, flush_bytes;
static uint64_t in_lat_us, in_lat_n, in_lat_max;
static int verbose;   /* COUCH_DEBUG=1: per-key logging, off by default */
static uint8_t *fb_map;          /* the visible page, mapped */
static uint8_t *ram_buf;         /* where LVGL draws (cached, so fast) */

/* Draw in cached RAM, copy the dirty rows into the visible page. No ioctl.
 *
 * Measured on this panel: an mmap write reaches the glass on its own, a
 * FBIOPAN_DISPLAY blocks ~17ms waiting for vsync, and a full-frame write() costs
 * ~98ms. So the cheapest correct thing is a plain memcpy of what changed -
 * about 0.2ms for a focus move.
 *
 * Two earlier conclusions here were wrong, both from testing with the backlight
 * off: that mmap writes are never displayed, and that 14MB/s was a hardware
 * ceiling. Neither is true; that was a self-inflicted copy. */
static void flush_cb(lv_display_t *disp, const lv_area_t *area, uint8_t *px_map)
{
    const size_t stride = fb_bytes / scr_h;
    int32_t y1 = area->y1 < 0 ? 0 : area->y1;
    int32_t y2 = area->y2 >= (int32_t)scr_h ? (int32_t)scr_h - 1 : area->y2;

    if (y2 >= y1) {
        const size_t off = (size_t)y1 * stride;
        const size_t len = (size_t)(y2 - y1 + 1) * stride;

        /* mtkfb composites ARGB8888 and LVGL leaves alpha clear, which would
         * render the frame invisible. */
        uint32_t *px = (uint32_t *)(px_map + off);
        for (size_t i = 0; i < len / 4; i++) px[i] |= 0xFF000000u;

        struct timespec a, b;
        clock_gettime(CLOCK_MONOTONIC, &a);
        memcpy(fb_map + off, px_map + off, len);
        clock_gettime(CLOCK_MONOTONIC, &b);
        flush_us += (uint64_t)(b.tv_sec - a.tv_sec) * 1000000 + (b.tv_nsec - a.tv_nsec) / 1000;
        flush_bytes += len;
    }

    if (lv_display_flush_is_last(disp)) flush_n++;
    lv_display_flush_ready(disp);
}

/* ---- a small remote-shaped UI, so the demo shows the real interaction ---- */

static lv_obj_t *status;

static void row_clicked(lv_event_t *e)
{
    const char *name = lv_event_get_user_data(e);
    lv_label_set_text_fmt(status, "selected: %s", name);
}

static lv_obj_t *make_row(lv_obj_t *parent, const char *icon, const char *name,
                          const char *value, lv_group_t *group)
{
    lv_obj_t *row = lv_button_create(parent);
    lv_obj_set_width(row, LV_PCT(100));
    lv_obj_set_height(row, 56);
    lv_obj_set_style_bg_color(row, lv_color_hex(0x171a21), 0);
    lv_obj_set_style_bg_color(row, lv_color_hex(0x25406b), LV_STATE_FOCUSED);
    lv_obj_set_style_radius(row, 10, 0);
    lv_obj_set_style_border_width(row, 0, 0);
    lv_obj_set_style_pad_hor(row, 12, 0);
    /* The focus fade is back on (LV_THEME_DEFAULT_TRANSITION_TIME in lv_conf.h).
     * It renders ~7 frames per press, but a frame costs ~124us, so the cost is
     * the 80ms settle rather than any real work. The sluggishness blamed on it
     * was actually the keypad driver: debounce-delay-ms = 50 plus
     * linux,no-autorepeat.
     *
     * Note: do not disable it with lv_obj_set_style_transition(obj, NULL, ...) -
     * a NULL descriptor crashes LVGL during build_ui. */

    lv_obj_t *l = lv_label_create(row);
    lv_label_set_text_fmt(l, "%s  %s", icon, name);
    lv_obj_set_style_text_color(l, lv_color_hex(0xe6e9ef), 0);
    lv_obj_align(l, LV_ALIGN_LEFT_MID, 0, 0);

    if (value) {
        lv_obj_t *v = lv_label_create(row);
        lv_label_set_text(v, value);
        lv_obj_set_style_text_color(v, lv_color_hex(0x7ce09b), 0);
        lv_obj_align(v, LV_ALIGN_RIGHT_MID, 0, 0);
    }

    lv_obj_add_event_cb(row, row_clicked, LV_EVENT_CLICKED, (void *)name);
    lv_group_add_obj(group, row);
    return row;
}

/* ---- keypad, read directly ------------------------------------------------
 *
 * LVGL's evdev driver binds both keypad nodes without error yet never delivers
 * a key, while a plain reader on the same node sees KEY_UP/KEY_DOWN fine. Rather
 * than keep bisecting the driver, read the devices here and inject into the
 * focus group. That is also what this device needs eventually: the remote has
 * colour buttons, Home, channel and volume keys that want their own meanings,
 * not just the six codes LVGL maps. */
#define MAX_KPD 3
static int kpd_fd[MAX_KPD];
static int kpd_n;

/* Key repeat, synthesised.
 *
 * The device tree sets linux,no-autorepeat on this keypad, so holding a button
 * yields one press and nothing until release - the gesture people actually use
 * to scroll a list does nothing. It also sets debounce-delay-ms = 50, so rapid
 * tapping is rate-limited in the driver and roughly half of fast presses never
 * reach userspace at all.
 *
 * Android synthesises repeat in its input framework rather than relying on the
 * driver, which is why it felt better here. Do the same: hold to scroll. */
#define REPEAT_DELAY_MS  400    /* before the first repeat  */
#define REPEAT_RATE_MS    70    /* between repeats after it */
static uint32_t held_key, held_since, last_repeat;

static void keypad_open(const char *path)
{
    if (kpd_n >= MAX_KPD) return;
    int fd = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) { printf("couch-gui: keypad %s: %s\n", path, strerror(errno)); return; }
    kpd_fd[kpd_n++] = fd;
    printf("couch-gui: keypad %s open\n", path);
}

static uint32_t map_key(int code)
{
    /* LV_KEY_UP/DOWN go to the *focused widget* - they adjust a slider or
     * scroll a list. Moving the focus between widgets is LV_KEY_PREV/NEXT.
     * Sending UP to a button therefore does exactly nothing, which is what
     * made the D-pad look dead. LVGL's own evdev driver maps KEY_UP to
     * LV_KEY_UP for the same reason, so it had the same effect.
     *
     * On a remote, up/down should walk the list, so map them to PREV/NEXT and
     * leave left/right as in-widget adjustment (slider, etc.). */
    switch (code) {
    case KEY_UP:                        return LV_KEY_PREV;
    case KEY_DOWN:                      return LV_KEY_NEXT;
    case KEY_LEFT:                      return LV_KEY_LEFT;
    case KEY_RIGHT:                     return LV_KEY_RIGHT;
    case KEY_ENTER: case KEY_OK:
    case KEY_SELECT: case BTN_LEFT:     return LV_KEY_ENTER;
    case KEY_BACK:  case KEY_ESC:       return LV_KEY_ESC;
    case KEY_VOLUMEUP: case KEY_CHANNELUP:     return LV_KEY_PREV;
    case KEY_VOLUMEDOWN: case KEY_CHANNELDOWN: return LV_KEY_NEXT;
    default:                            return 0;
    }
}

/* Focus traversal lives in LVGL's indev layer (indev_keypad_proc), not in
 * lv_group_send_data - that only delivers a key to the already-focused widget.
 * Reading the devices ourselves means doing the traversal ourselves too. */
static void apply_key(lv_group_t *group, uint32_t k)
{
    if (k == LV_KEY_NEXT) {
        lv_group_focus_next(group);
    } else if (k == LV_KEY_PREV) {
        lv_group_focus_prev(group);
    } else if (k == LV_KEY_ENTER) {
        lv_obj_t *f = lv_group_get_focused(group);   /* buttons want press/release */
        if (f) {
            lv_obj_send_event(f, LV_EVENT_PRESSED, NULL);
            lv_obj_send_event(f, LV_EVENT_RELEASED, NULL);
            lv_obj_send_event(f, LV_EVENT_CLICKED, NULL);
        }
    } else {
        lv_group_send_data(group, k);
    }
}

static int keypad_poll(lv_group_t *group)
{
    int got = 0;
    struct input_event ev;
    for (int i = 0; i < kpd_n; i++) {
        while (read(kpd_fd[i], &ev, sizeof ev) == (ssize_t)sizeof ev) {
            if (ev.type != EV_KEY) continue;
            if (ev.value == 0) {                 /* release: stop repeating */
                if (map_key(ev.code) == held_key) held_key = 0;
                continue;
            }
            if (ev.value != 1) continue;         /* 2 = driver repeat, unused here */
            uint32_t k = map_key(ev.code);

            /* Latency from when the kernel timestamped the press to now: this
             * separates "slow to receive" from "slow to draw". */
            struct timeval now;
            gettimeofday(&now, NULL);
            uint64_t lat = (uint64_t)(now.tv_sec - ev.time.tv_sec) * 1000000
                         + (now.tv_usec - ev.time.tv_usec);
            if (lat < 5000000) {
                in_lat_us += lat; in_lat_n++;
                if (lat > in_lat_max) in_lat_max = lat;
            }
            if (verbose) printf("couch-gui: code=%d -> lv_key=0x%02x (%llu us old)\n",
                                ev.code, (unsigned)k, (unsigned long long)lat);
            if (!k) continue;

            held_key = k;                        /* arm repeat while held */
            held_since = millis();
            last_repeat = 0;

            apply_key(group, k);
            got = 1;
        }
    }
    /* Synthesised repeat while a key is held. */
    if (held_key) {
        uint32_t now = millis();
        if (now - held_since >= REPEAT_DELAY_MS &&
            (last_repeat == 0 || now - last_repeat >= REPEAT_RATE_MS)) {
            apply_key(group, held_key);
            last_repeat = now;
            got = 1;
        }
    }
    return got;
}

static void build_ui(lv_group_t *group)
{
    lv_obj_t *scr = lv_screen_active();
    lv_obj_set_style_bg_color(scr, lv_color_hex(0x0a0c12), 0);
    lv_obj_set_style_pad_all(scr, 14, 0);

    lv_obj_t *title = lv_label_create(scr);
    lv_label_set_text(title, "Living Room");
    lv_obj_set_style_text_color(title, lv_color_hex(0xffffff), 0);
    lv_obj_set_style_text_font(title, &lv_font_montserrat_28, 0);
    lv_obj_align(title, LV_ALIGN_TOP_LEFT, 0, 0);

    lv_obj_t *list = lv_obj_create(scr);
    lv_obj_remove_style_all(list);
    lv_obj_set_size(list, LV_PCT(100), 430);
    lv_obj_align(list, LV_ALIGN_TOP_LEFT, 0, 52);
    lv_obj_set_flex_flow(list, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_style_pad_row(list, 8, 0);
    lv_obj_set_scrollbar_mode(list, LV_SCROLLBAR_MODE_OFF);

    make_row(list, LV_SYMBOL_VIDEO, "Watch TV",      NULL,     group);
    make_row(list, LV_SYMBOL_POWER, "All Lights",    "off",    group);
    make_row(list, LV_SYMBOL_POWER, "Ceiling Only",  "off",    group);
    make_row(list, LV_SYMBOL_SETTINGS, "Roller Blind", "open", group);

    lv_obj_t *sl = lv_slider_create(scr);
    lv_obj_set_width(sl, LV_PCT(100));
    lv_obj_align(sl, LV_ALIGN_BOTTOM_MID, 0, -52);
    lv_slider_set_value(sl, 60, LV_ANIM_OFF);
    lv_group_add_obj(group, sl);

    status = lv_label_create(scr);
    lv_label_set_text(status, "keys navigate - touch also works");
    lv_obj_set_style_text_color(status, lv_color_hex(0x98a2b3), 0);
    lv_obj_align(status, LV_ALIGN_BOTTOM_LEFT, 0, 0);
}

static void key_seen(lv_event_t *e)
{
    uint32_t k = lv_event_get_key(e);
    printf("couch-gui: LVGL delivered key 0x%02x\n", (unsigned)k);
    lv_label_set_text_fmt(status, "key 0x%02x", (unsigned)k);
}

/* Log the focused object each time it changes, so we can tell "no keys" from
 * "keys arrive but focus is stuck" from "focus moves but nothing repaints". */
static void focus_changed(lv_group_t *g)
{
    if (verbose) printf("couch-gui: focus -> %p\n", (void *)lv_group_get_focused(g));
}

int main(void)
{
    struct fb_var_screeninfo var;
    struct fb_fix_screeninfo fix;

    verbose = getenv("COUCH_DEBUG") != NULL;
    /* Line buffering, not unbuffered: unbuffered means a write() syscall to
     * eMMC for every printf, inside the input loop, which is itself enough to
     * make the UI feel sluggish. */
    setvbuf(stdout, NULL, _IOLBF, 0);

    fbfd = open("/dev/fb0", O_RDWR);
    if (fbfd < 0) { perror("open /dev/fb0"); return 1; }
    if (ioctl(fbfd, FBIOGET_VSCREENINFO, &var) < 0) { perror("VSCREENINFO"); return 1; }
    if (ioctl(fbfd, FBIOGET_FSCREENINFO, &fix) < 0) { perror("FSCREENINFO"); return 1; }

    scr_w = var.xres;
    scr_h = var.yres;
    fb_bytes = (size_t)(fix.line_length ? fix.line_length : scr_w * 4) * scr_h;
    printf("couch-gui: %ux%u %ubpp, %zu bytes/frame\n", scr_w, scr_h,
           var.bits_per_pixel, fb_bytes);

    backlight_on();
    lv_init();
    lv_tick_set_cb(millis);

    fb_map = mmap(NULL, fb_bytes, PROT_READ | PROT_WRITE, MAP_SHARED, fbfd, 0);
    if (fb_map == MAP_FAILED) { perror("mmap"); return 1; }

    ram_buf = malloc(fb_bytes);          /* cached: drawing here is far faster */
    if (!ram_buf) { perror("malloc"); return 1; }
    printf("couch-gui: drawing in RAM, copying dirty rows to the visible page (no ioctl)\n");

    lv_display_t *disp = lv_display_create(scr_w, scr_h);
    lv_display_set_color_format(disp, LV_COLOR_FORMAT_ARGB8888);
    lv_display_set_buffers(disp, ram_buf, NULL, fb_bytes, LV_DISPLAY_RENDER_MODE_DIRECT);
    lv_display_set_flush_cb(disp, flush_cb);

    lv_group_t *group = lv_group_create();
    lv_group_set_default(group);

    lv_indev_t *touch = lv_evdev_create(LV_INDEV_TYPE_POINTER, TOUCH_DEV);
    if (!touch) printf("couch-gui: no touch on %s\n", TOUCH_DEV);

    keypad_open("/dev/input/event1");   /* mt_gpio_kpd */
    keypad_open("/dev/input/event2");   /* mtk-kpd     */
    printf("couch-gui: %d keypad device(s)\n", kpd_n);

    build_ui(group);
    /* LV_EVENT_KEY fires on the focused object, so a callback on the screen
     * never sees anything - the screen is not focusable. */
    lv_group_set_focus_cb(group, focus_changed);
    lv_group_focus_next(group);
    printf("couch-gui: ui built, entering loop\n");

    uint32_t last_bl = 0, last_stat = 0;
    for (;;) {
        int had_input = keypad_poll(group);
        lv_timer_handler();
        /* Render straight away on input instead of waiting for the refresh
         * tick; otherwise every press pays up to a frame of latency. */
        if (had_input) lv_refr_now(NULL);
        usleep(had_input ? 1000 : 5000);

        if (millis() - last_stat > 5000 && flush_n) {
            printf("couch-gui: %llu frames, %llu KB, %llu us/frame | input %llu us avg, %llu us max (n=%llu)\n",
                   (unsigned long long)flush_n, (unsigned long long)(flush_bytes / 1024),
                   (unsigned long long)(flush_us / flush_n),
                   (unsigned long long)(in_lat_n ? in_lat_us / in_lat_n : 0),
                   (unsigned long long)in_lat_max, (unsigned long long)in_lat_n);
            flush_us = flush_n = flush_bytes = 0;
            in_lat_us = in_lat_n = in_lat_max = 0;
            last_stat = millis();
        }
        /* The panel switches its own backlight off when idle. */
        if (millis() - last_bl > 5000) { backlight_on(); last_bl = millis(); }
    }
    return 0;
}