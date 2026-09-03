/*
 * Print raw evdev key events, so we can see what the remote's buttons actually
 * report and which node they arrive on.
 *
 * LVGL's evdev keypad driver only maps a handful of codes (KEY_UP/DOWN/LEFT/
 * RIGHT/ENTER/ESC/DEL/BACKSPACE). Anything else - BTN_* gamepad codes, for
 * instance - arrives and is silently dropped, which looks exactly like the
 * keypad not working.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/select.h>
#include <linux/input.h>

static const char *keyname(int code)
{
    switch (code) {
    case KEY_UP:        return "KEY_UP";
    case KEY_DOWN:      return "KEY_DOWN";
    case KEY_LEFT:      return "KEY_LEFT";
    case KEY_RIGHT:     return "KEY_RIGHT";
    case KEY_ENTER:     return "KEY_ENTER";
    case KEY_ESC:       return "KEY_ESC";
    case KEY_BACK:      return "KEY_BACK";
    case KEY_HOME:      return "KEY_HOME";
    case KEY_HOMEPAGE:  return "KEY_HOMEPAGE";
    case KEY_MENU:      return "KEY_MENU";
    case KEY_POWER:     return "KEY_POWER";
    case KEY_VOLUMEUP:  return "KEY_VOLUMEUP";
    case KEY_VOLUMEDOWN:return "KEY_VOLUMEDOWN";
    case KEY_MUTE:      return "KEY_MUTE";
    case KEY_OK:        return "KEY_OK";
    case KEY_SELECT:    return "KEY_SELECT";
    case KEY_CHANNELUP: return "KEY_CHANNELUP";
    case KEY_CHANNELDOWN: return "KEY_CHANNELDOWN";
    case KEY_RED:       return "KEY_RED";
    case KEY_GREEN:     return "KEY_GREEN";
    case KEY_YELLOW:    return "KEY_YELLOW";
    case KEY_BLUE:      return "KEY_BLUE";
    case BTN_A:         return "BTN_A";
    case BTN_B:         return "BTN_B";
    case BTN_X:         return "BTN_X";
    case BTN_Y:         return "BTN_Y";
    case KEY_F1:        return "KEY_F1";
    case KEY_F2:        return "KEY_F2";
    case KEY_F3:        return "KEY_F3";
    case KEY_F4:        return "KEY_F4";
    case KEY_F5:        return "KEY_F5";
    case KEY_F6:        return "KEY_F6";
    case KEY_F7:        return "KEY_F7";
    case KEY_F8:        return "KEY_F8";
    case KEY_F9:        return "KEY_F9";
    case KEY_F10:       return "KEY_F10";
    case KEY_F11:       return "KEY_F11";
    case KEY_F12:       return "KEY_F12";
    default:            return "?";
    }
}

int main(int argc, char **argv)
{
    int fds[8], n = 0;
    char names[8][64];

    if (argc > 1) {
        for (int i = 1; i < argc && n < 8; i++) {
            fds[n] = open(argv[i], O_RDONLY);
            if (fds[n] < 0) { printf("cannot open %s: %s\n", argv[i], strerror(errno)); continue; }
            snprintf(names[n], sizeof names[n], "%s", argv[i]);
            n++;
        }
    } else {
        for (int i = 0; i < 4 && n < 8; i++) {
            char p[32];
            snprintf(p, sizeof p, "/dev/input/event%d", i);
            fds[n] = open(p, O_RDONLY);
            if (fds[n] < 0) continue;
            snprintf(names[n], sizeof names[n], "%s", p);
            n++;
        }
    }
    if (!n) { printf("no input devices opened\n"); return 1; }
    printf("watching %d device(s) - press buttons\n", n);
    fflush(stdout);

    for (;;) {
        fd_set r;
        FD_ZERO(&r);
        int max = 0;
        for (int i = 0; i < n; i++) { FD_SET(fds[i], &r); if (fds[i] > max) max = fds[i]; }
        if (select(max + 1, &r, NULL, NULL, NULL) <= 0) continue;
        for (int i = 0; i < n; i++) {
            if (!FD_ISSET(fds[i], &r)) continue;
            struct input_event ev;
            if (read(fds[i], &ev, sizeof ev) != (ssize_t)sizeof ev) continue;
            if (ev.type != EV_KEY) continue;
            printf("%-20s code=%-4d %-16s %s\n", names[i], ev.code, keyname(ev.code),
                   ev.value ? "press" : "release");
            fflush(stdout);
        }
    }
}
