/* Do Android property reads work in this environment?
 *
 * Android 8's libEGL loader builds the driver filename from ro.hardware.egl /
 * ro.board.platform, so if properties are not readable it can never find
 * libGLES_mali.so no matter which directories exist. stage2 restores a
 * property snapshot into /dev/__properties__; this checks it is actually
 * usable by bionic's reader, not merely present as files. */
#include <stdio.h>
#include <string.h>

extern int __system_property_get(const char *name, char *value);

int main(void)
{
    static const char *keys[] = {
        "ro.hardware", "ro.hardware.egl", "ro.board.platform", "ro.product.board",
        "ro.opengles.version", "ro.build.version.sdk", "persist.mtk.wcn.combo.chipid",
    };
    char v[92];
    int ok = 0;
    for (unsigned i = 0; i < sizeof keys / sizeof keys[0]; i++) {
        v[0] = 0;
        int n = __system_property_get(keys[i], v);
        printf("  %-30s %s%s\n", keys[i], n > 0 ? "= " : "(unset)", n > 0 ? v : "");
        if (n > 0) ok++;
    }
    printf("\n  %d of %u properties readable\n", ok, (unsigned)(sizeof keys / sizeof keys[0]));
    return ok > 0 ? 0 : 1;
}
