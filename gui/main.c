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
#include "theme.h"
#include "icons.h"

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

        /* Two fixes in the pass we already make, so they cost nothing extra:
         *
         * 1. Alpha. LVGL leaves the alpha byte clear and mtkfb composites
         *    ARGB8888, so an untouched frame is fully transparent.
         * 2. Channel order. The panel reads the low byte as red - it wants
         *    ABGR while LVGL renders ARGB. Verified with labelled bars: a
         *    stored 0xffff0000 (pure red) displays blue. Note fb_var_screeninfo
         *    reports red=0/8 blue=16/8, which is the same claim, but I got this
         *    backwards once by trusting a washed-out grey to tell me - only a
         *    saturated test pattern settles it. */
        uint32_t *px = (uint32_t *)(px_map + off);
        for (size_t i = 0; i < len / 4; i++) {
            uint32_t v = px[i];
            px[i] = 0xFF000000u | ((v & 0x0000FFu) << 16)
                                | (v & 0x00FF00u)
                                | ((v >> 16) & 0x0000FFu);
        }

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

/* ---- battery ------------------------------------------------------------
 * The gauge is the kernel's, via /sys/class/power_supply. "status" is the
 * authoritative charging signal: usb/online only reports that a cable is
 * present, which it always is while the remote sits on a bench being
 * debugged, so it would read as charging forever. */
#define PS_BATTERY "/sys/class/power_supply/battery/"

static lv_obj_t *batt_icon, *batt_lbl;

static int read_first_line(const char *path, char *buf, size_t n)
{
    FILE *f = fopen(path, "r");
    if (!f) return 0;
    char *got = fgets(buf, (int)n, f);
    fclose(f);
    if (!got) return 0;
    buf[strcspn(buf, "\r\n")] = 0;
    return 1;
}

static void battery_tick(lv_timer_t *t)
{
    char cap_s[32], st[32] = "";
    (void)t;
    if (!batt_lbl) return;

    /* No gauge is better than a wrong one: hide the readout rather than
     * showing a stale or invented figure. */
    if (!read_first_line(PS_BATTERY "capacity", cap_s, sizeof cap_s)) {
        if (verbose) printf("couch-gui: no battery gauge at %s\n", PS_BATTERY);
        lv_obj_add_flag(batt_lbl, LV_OBJ_FLAG_HIDDEN);
        lv_obj_add_flag(batt_icon, LV_OBJ_FLAG_HIDDEN);
        return;
    }
    int cap = atoi(cap_s);
    if (cap < 0)   cap = 0;
    if (cap > 100) cap = 100;

    read_first_line(PS_BATTERY "status", st, sizeof st);
    int charging = (strcmp(st, "Charging") == 0 || strcmp(st, "Full") == 0);
    if (verbose) printf("couch-gui: battery %d%% \"%s\" charging=%d\n", cap, st, charging);

    lv_image_set_src(batt_icon, charging   ? &icon_battery_charging
                              : cap >= 70  ? &icon_battery_full
                              : cap >= 35  ? &icon_battery_medium
                                           : &icon_battery_low);
    lv_obj_set_style_image_recolor(batt_icon,
        lv_color_hex(!charging && cap <= 15 ? C_DESTRUCTIVE : C_MUTED_FOREGROUND), 0);
    lv_label_set_text_fmt(batt_lbl, "%d%%", cap);
    lv_obj_remove_flag(batt_lbl, LV_OBJ_FLAG_HIDDEN);
    lv_obj_remove_flag(batt_icon, LV_OBJ_FLAG_HIDDEN);
}

static lv_obj_t *status;
static lv_obj_t *clock_lbl;

/* Tick the clock once a second. LVGL timers run off lv_timer_handler, which the
 * main loop already calls, so this needs no thread. */
/* Seconds east of UTC, taken from the system rather than from libc.
 *
 * couch-gui is a static bionic binary, and bionic resolves timezones through
 * Android's tzdata and persist.sys.timezone - it never reads Alpine's
 * /etc/localtime, so localtime_r() yields UTC. Its POSIX TZ parsing does not
 * apply the DST rule here either, with or without explicit transition times,
 * which leaves the clock an hour behind all summer.
 *
 * Alpine's own date(1) has correct tzdata, so ask it once an hour: that keeps
 * DST transitions right without shipping a TZif parser. */
static long tz_offset(void)
{
    static long cached;
    static time_t checked;
    time_t now = time(NULL);
    if (checked && now - checked < 3600) return cached;

    FILE *f = popen("date +%z", "r");
    if (f) {
        char b[16] = {0};
        if (fgets(b, sizeof b, f) && (b[0] == '+' || b[0] == '-')) {
            int hh = (b[1] - '0') * 10 + (b[2] - '0');
            int mm = (b[3] - '0') * 10 + (b[4] - '0');
            cached = (hh * 3600L + mm * 60L) * (b[0] == '-' ? -1 : 1);
        }
        pclose(f);
    }
    checked = now;
    return cached;
}

static void clock_tick(lv_timer_t *t)
{
    LV_UNUSED(t);
    time_t now = time(NULL) + tz_offset();
    struct tm tm;
    gmtime_r(&now, &tm);
    char buf[16];
    strftime(buf, sizeof buf, "%I:%M %p", &tm);
    /* strftime pads the hour to two digits; drop the leading zero so it reads
     * "1:40 PM" rather than "01:40 PM". */
    const char *txt = (buf[0] == '0') ? buf + 1 : buf;
    if (clock_lbl) lv_label_set_text(clock_lbl, txt);
}

static void row_clicked(lv_event_t *e)
{
    lv_label_set_text_fmt(status, "selected: %s", (const char *)lv_event_get_user_data(e));
}

static lv_obj_t *add_row(lv_obj_t *parent, const lv_image_dsc_t *icon,
                         const char *name, const char *value, lv_group_t *group)
{
    lv_obj_t *row = couch_row(parent, icon, name, value);
    lv_obj_add_event_cb(row, row_clicked, LV_EVENT_CLICKED, (void *)name);
    lv_group_add_obj(group, row);
    return row;
}

static void focus_changed(lv_group_t *g)
{
    if (verbose) printf("couch-gui: focus -> %p\n", (void *)lv_group_get_focused(g));
}

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

/* Boot splash: a pulsing sofa while the rest of the system settles.
 *
 * fbcon owns the panel until this point, so the log is what you see during
 * boot; couch-gui takes the screen over as soon as it starts. */
static void splash_opa(void *obj, int32_t v)
{
    lv_obj_set_style_image_opa((lv_obj_t *)obj, (lv_opa_t)v, 0);
}

static void build_ui(lv_group_t *group);

/* ---- setup screen -------------------------------------------------------
 * Shown instead of the room UI when the remote has no network of its own yet.
 * The QR carries a "WIFI:" join record, which phone cameras act on directly,
 * so nobody has to read an SSID off a 3.1" panel and type it in. Joining the
 * open setup network then trips the captive portal, which is where the user
 * picks the network the remote should actually live on. */
#define QR_PX   288
#define QR_PAD   16

/* Setup mode is a file, not an environment variable, so that it can be turned
 * off while the remote is running: stage2 supervises couch-gui in a restart
 * loop whose environment is fixed at boot, so an env flag could never be
 * cleared without killing the supervisor too. join.sh removes this marker and
 * restarts the GUI once the remote is on a real network. */
static int in_setup_mode(void)
{
    return getenv("COUCH_SETUP") != NULL || access("/tmp/couch.setup", F_OK) == 0;
}

static const char *setup_ssid(void)
{
    static char buf[64];
    const char *s = getenv("COUCH_SETUP_SSID");
    if (s && *s) return s;

    FILE *f = fopen("/tmp/portal.ssid", "r");
    if (f) {
        char *got = fgets(buf, sizeof buf, f);
        fclose(f);
        if (got) {
            buf[strcspn(buf, "\r\n")] = 0;
            if (buf[0]) return buf;
        }
    }
    return "Couch-Setup";
}

static void setup_line(lv_obj_t *scr, const char *label, const char *value,
                       const lv_font_t *font, int y)
{
    lv_obj_t *l = couch_muted(scr, label);
    lv_obj_align(l, LV_ALIGN_TOP_MID, 0, y);
    lv_obj_t *v = lv_label_create(scr);
    lv_label_set_text(v, value);
    lv_obj_set_style_text_color(v, lv_color_hex(C_FOREGROUND), 0);
    lv_obj_set_style_text_font(v, font, 0);
    lv_obj_align(v, LV_ALIGN_TOP_MID, 0, y + 26);
}

/* The portal demands a physical button press before it grants SSH access, but
 * confirm.sh - which does the actual waiting - has no way to say so on the
 * panel: its output goes to a log, and in setup mode the GUI owns the screen.
 * Without this the page tells you to press a button while the remote itself
 * shows nothing. Driven by the very files confirm.sh already uses. */
static lv_obj_t *approve_card, *approve_text;
static uint32_t approve_hide_at;

static void approve_show(const char *msg, uint32_t hold_ms)
{
    lv_label_set_text(approve_text, msg);
    lv_obj_remove_flag(approve_card, LV_OBJ_FLAG_HIDDEN);
    approve_hide_at = hold_ms ? millis() + hold_ms : 0;
}

static void approve_tick(lv_timer_t *t)
{
    static int waiting;
    (void)t;

    if (access("/tmp/press.request", F_OK) == 0) {
        if (!waiting) {
            waiting = 1;
            remove("/tmp/press.result");     /* stale verdict from last time */
            approve_show("Press any button\nto approve SSH access", 0);
        }
        return;
    }

    if (waiting) {                           /* the request just went away */
        char verdict[16] = "";
        waiting = 0;
        read_first_line("/tmp/press.result", verdict, sizeof verdict);
        approve_show(strcmp(verdict, "ok") == 0 ? "SSH access approved"
                                                : "Approval timed out", 3000);
        return;
    }

    if (approve_hide_at && millis() >= approve_hide_at) {
        approve_hide_at = 0;
        lv_obj_add_flag(approve_card, LV_OBJ_FLAG_HIDDEN);
    }
}

static void build_setup_ui(void)
{
    couch_theme_init();
    lv_obj_t *scr = lv_screen_active();

    lv_obj_t *h = couch_h1(scr, "Wi-Fi Setup");
    lv_obj_align(h, LV_ALIGN_TOP_MID, 0, 28);

    lv_obj_t *sub = couch_muted(scr, "Scan with your phone camera");
    lv_obj_align(sub, LV_ALIGN_TOP_MID, 0, 68);

    /* A QR only scans reliably against a light quiet zone, so this one card
     * stays white in an otherwise near-black theme. */
    lv_obj_t *card = lv_obj_create(scr);
    lv_obj_remove_style_all(card);
    lv_obj_remove_flag(card, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(card, QR_PX + 2 * QR_PAD, QR_PX + 2 * QR_PAD);
    lv_obj_set_style_bg_color(card, lv_color_hex(0xFFFFFF), 0);
    lv_obj_set_style_bg_opa(card, LV_OPA_COVER, 0);
    lv_obj_set_style_radius(card, R_LG, 0);
    lv_obj_align(card, LV_ALIGN_TOP_MID, 0, 108);

    /* T:nopass because the setup network is open, so the scan joins outright
     * instead of prompting for a passphrase nobody has. */
    char join[160];
    snprintf(join, sizeof join, "WIFI:S:%s;T:nopass;;", setup_ssid());

    lv_obj_t *qr = lv_qrcode_create(card);
    lv_qrcode_set_size(qr, QR_PX);
    lv_qrcode_set_dark_color(qr, lv_color_hex(0x000000));
    lv_qrcode_set_light_color(qr, lv_color_hex(0xFFFFFF));
    if (lv_qrcode_update(qr, join, strlen(join)) != LV_RESULT_OK)
        fprintf(stderr, "qrcode: could not encode \"%s\"\n", join);
    lv_obj_center(qr);

    /* Everything the QR encodes, spelled out for a phone that will not scan
     * it - an older camera app, or a reader that ignores WIFI: records. */
    setup_line(scr, "Network - no password", setup_ssid(),
               &lv_font_montserrat_28, 468);
    setup_line(scr, "Then open", "http://192.168.4.1",
               &lv_font_montserrat_20, 556);

    /* Hidden until the portal asks for an approval. Inverted against the dark
     * theme so it reads as a demand for attention, not another line of text. */
    approve_card = lv_obj_create(scr);
    lv_obj_remove_style_all(approve_card);
    lv_obj_remove_flag(approve_card, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(approve_card, 432, 116);
    lv_obj_set_style_bg_color(approve_card, lv_color_hex(C_PRIMARY), 0);
    lv_obj_set_style_bg_opa(approve_card, LV_OPA_COVER, 0);
    lv_obj_set_style_radius(approve_card, R_LG, 0);
    lv_obj_align(approve_card, LV_ALIGN_BOTTOM_MID, 0, -28);
    lv_obj_add_flag(approve_card, LV_OBJ_FLAG_HIDDEN);

    approve_text = lv_label_create(approve_card);
    lv_label_set_text(approve_text, "");
    lv_obj_set_style_text_color(approve_text, lv_color_hex(C_PRIMARY_FG), 0);
    lv_obj_set_style_text_font(approve_text, &lv_font_montserrat_20, 0);
    lv_obj_set_style_text_align(approve_text, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_center(approve_text);

    lv_timer_create(approve_tick, 400, NULL);
}

static void splash_done(lv_timer_t *t)
{
    lv_group_t *group = lv_timer_get_user_data(t);
    lv_anim_delete_all();
    lv_obj_clean(lv_screen_active());
    if (in_setup_mode()) build_setup_ui();
    else                 build_ui(group);
}

static void show_splash(lv_group_t *group)
{
    couch_theme_init();
    lv_obj_t *im = lv_image_create(lv_screen_active());
    lv_image_set_src(im, &icon_sofa_lg);
    lv_obj_set_style_image_recolor(im, lv_color_hex(C_FOREGROUND), 0);
    lv_obj_set_style_image_recolor_opa(im, LV_OPA_COVER, 0);
    lv_obj_center(im);

    lv_anim_t a;
    lv_anim_init(&a);
    lv_anim_set_var(&a, im);
    lv_anim_set_exec_cb(&a, splash_opa);
    lv_anim_set_values(&a, LV_OPA_40, LV_OPA_COVER);
    lv_anim_set_duration(&a, 800);
    lv_anim_set_playback_duration(&a, 800);
    lv_anim_set_repeat_count(&a, LV_ANIM_REPEAT_INFINITE);
    lv_anim_start(&a);

    lv_timer_t *t = lv_timer_create(splash_done, 2400, group);
    lv_timer_set_repeat_count(t, 1);
}

static void build_ui(lv_group_t *group)
{
    couch_theme_init();
    lv_obj_t *scr = lv_screen_active();

    if (getenv("COUCH_BARS")) {
        static const uint32_t probe[] = { 0xFF0000, 0x00FF00, 0x0000FF };
        static const char *pname[] = { "RED", "GREEN", "BLUE" };
        for (int i = 0; i < 3; i++) {
            lv_obj_t *bar = lv_obj_create(scr);
            lv_obj_remove_style_all(bar);
            lv_obj_set_size(bar, 140, 80);
            lv_obj_set_pos(bar, i * 148, 0);
            lv_obj_set_style_bg_color(bar, lv_color_hex(probe[i]), 0);
            lv_obj_set_style_bg_opa(bar, LV_OPA_COVER, 0);
            lv_obj_t *t = lv_label_create(bar);
            lv_label_set_text(t, pname[i]);
            lv_obj_set_style_text_color(t, lv_color_hex(0x000000), 0);
            lv_obj_center(t);
        }
    }

    int top = getenv("COUCH_BARS") ? 88 : 0;

    lv_obj_t *h = couch_h1(scr, "Living Room");
    lv_obj_align(h, LV_ALIGN_TOP_LEFT, 0, top);

    /* Battery then clock, in a row: the clock's width changes between "9:05 PM"
     * and "12:05 PM", and a row keeps that from shifting the battery about. */
    lv_obj_t *statusbar = lv_obj_create(scr);
    lv_obj_remove_style_all(statusbar);
    lv_obj_remove_flag(statusbar, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(statusbar, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(statusbar, LV_FLEX_FLOW_ROW);
    /* START on the main axis, not END: with LV_SIZE_CONTENT the container
     * measures itself from the flow, and END makes it collapse to the width of
     * the last child, laying the earlier ones out at negative x where they are
     * clipped against the parent and vanish. The row is put on the right by
     * aligning the container itself, below. */
    lv_obj_set_flex_align(statusbar, LV_FLEX_ALIGN_START,
                          LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_column(statusbar, 5, 0);
    lv_obj_align(statusbar, LV_ALIGN_TOP_RIGHT, 0, top + 4);

    batt_icon = lv_image_create(statusbar);
    lv_image_set_src(batt_icon, &icon_battery_full);
    lv_obj_set_style_image_recolor_opa(batt_icon, LV_OPA_COVER, 0);

    batt_lbl = lv_label_create(statusbar);
    lv_label_set_text(batt_lbl, "");
    lv_obj_set_style_text_color(batt_lbl, lv_color_hex(C_MUTED_FOREGROUND), 0);
    lv_obj_set_style_text_font(batt_lbl, &lv_font_montserrat_20, 0);

    clock_lbl = lv_label_create(statusbar);
    lv_label_set_text(clock_lbl, "--:--");
    lv_obj_set_style_text_color(clock_lbl, lv_color_hex(C_MUTED_FOREGROUND), 0);
    lv_obj_set_style_text_font(clock_lbl, &lv_font_montserrat_20, 0);
    lv_obj_set_style_pad_left(clock_lbl, 9, 0);

    lv_timer_create(clock_tick, 1000, NULL);
    clock_tick(NULL);
    /* The gauge moves in percent, not seconds. */
    lv_timer_create(battery_tick, 15000, NULL);
    battery_tick(NULL);

    lv_obj_t *card = couch_group(scr);
    lv_obj_set_size(card, LV_PCT(100), 4 * (ROW_H + GAP) + 2 * GAP);
    /* Same offset as when a subtitle sat here - the line is gone, the spacing
     * it created is deliberate. */
    lv_obj_align(card, LV_ALIGN_TOP_LEFT, 0, top + 72);

    add_row(card, &icon_tv,         "Watch TV",     NULL,   group);
    add_row(card, &icon_lightbulb,  "All Lights",   "off",  group);
    add_row(card, &icon_lightbulb,  "Ceiling Only", "off",  group);
    add_row(card, &icon_blinds,     "Roller Blind", "open", group);

    lv_obj_t *actions = lv_obj_create(scr);
    lv_obj_remove_style_all(actions);
    /* Taller and padded so the focus ring, which draws outside each button,
     * is not clipped by this container on any side. */
    lv_obj_set_size(actions, LV_PCT(100), 52 + 2 * 8);
    /* Sits at the bottom now the hint line is gone; the container's own 8px
     * padding keeps the focus ring clear of the screen edge. */
    lv_obj_align(actions, LV_ALIGN_BOTTOM_MID, 0, 0);
    lv_obj_set_flex_flow(actions, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(actions, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER,
                          LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_all(actions, 8, 0);
    lv_obj_set_style_pad_column(actions, GAP + 4, 0);

    lv_obj_t *b1 = couch_button(actions, BTN_DEFAULT, "All Off");
    lv_obj_set_height(b1, 52);
    lv_obj_set_flex_grow(b1, 1);
    lv_group_add_obj(group, b1);
    lv_obj_t *b2 = couch_button(actions, BTN_OUTLINE, "Scenes");
    lv_obj_set_height(b2, 52);
    lv_obj_set_flex_grow(b2, 1);
    lv_group_add_obj(group, b2);

    status = couch_muted(scr, "");
    lv_obj_add_flag(status, LV_OBJ_FLAG_HIDDEN);
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

    /* Claim the panel before drawing anything: fbcon keeps printing boot
     * output otherwise, and since we only repaint what changed, its text
     * survives on top of the UI instead of being covered by the splash. */
    close(open("/tmp/couch.gui", O_WRONLY | O_CREAT | O_TRUNC, 0644));

    /* Then clear the panel ourselves. LVGL invalidates only what it drew - for
     * the splash, the rows holding one centred icon - so the boot log survived
     * everywhere else and the splash appeared to float on top of it. Paint the
     * background over the whole framebuffer once, in the panel's own channel
     * order (low byte red), so the handover is clean whatever LVGL redraws. */
    {
        uint32_t bg = 0xFF000000u
                    | ((C_BACKGROUND & 0x0000FFu) << 16)   /* blue  -> high  */
                    | (C_BACKGROUND & 0x00FF00u)           /* green stays     */
                    | ((C_BACKGROUND >> 16) & 0x0000FFu);  /* red   -> low   */
        uint32_t *p = (uint32_t *)fb_map;
        for (size_t i = 0; i < fb_bytes / 4; i++) p[i] = bg;
    }

    show_splash(group);
    /* LV_EVENT_KEY fires on the focused object, so a callback on the screen
     * never sees anything - the screen is not focusable. */
    lv_group_set_focus_cb(group, focus_changed);
    lv_group_focus_next(group);
    /* COUCH_FOCUS=n advances focus n places at startup, so a screenshot can
     * show any focus position without someone pressing buttons. */
    if (getenv("COUCH_FOCUS")) {
        int n = atoi(getenv("COUCH_FOCUS"));
        for (int i = 0; i < n; i++) lv_group_focus_next(group);
    }
    printf("couch-gui: focused object = %p, group size = %u\n",
           (void *)lv_group_get_focused(group), lv_group_get_obj_count(group));
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