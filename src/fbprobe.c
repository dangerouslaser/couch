/*
 * Does this gralloc's framebuffer device actually work?
 *
 * gralloc.mt6580.so contains fb_post, but also the string "POST a ION_FB_HEAP
 * buffer, not implementation yet. Show the 1st frame instead." If posting is a
 * stub, the small GPU route - gralloc + Mali, no hwcomposer - has no way to get
 * a rendered buffer onto the panel, and the port needs the whole HWC stack with
 * binder and HIDL behind it. That is the difference between a bridge and a
 * project, so it is worth answering before anything else.
 *
 * No libhybris here. These blobs are bionic ELFs and this is an Android kernel,
 * so a bionic executable runs natively - stage2 already does exactly this for
 * wmt_loader and wpa_supplicant. libhybris is only needed later, to let a musl
 * Rust binary link against them.
 *
 * The module is loaded by dlopen + the HAL_MODULE_INFO_SYM ("HMI") symbol
 * rather than through libhardware, which avoids hw_get_module's property
 * lookups and lets us name the .so explicitly.
 *
 * The AOSP structure layouts are reproduced below; the NDK does not ship
 * hardware/gralloc.h. They have been stable across Android releases, but a
 * mismatch here reads as a crash or nonsense field values rather than a clean
 * error, so the field dump is worth sanity-checking before trusting a result.
 */
#include <dlfcn.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

typedef struct hw_module_t hw_module_t;
typedef struct hw_device_t hw_device_t;

typedef struct { int (*open)(const hw_module_t *, const char *, hw_device_t **); }
        hw_module_methods_t;

struct hw_module_t {
    uint32_t tag;                       /* 'HWMT' */
    uint16_t module_api_version;
    uint16_t hal_api_version;
    const char *id;
    const char *name;
    const char *author;
    hw_module_methods_t *methods;
    void *dso;
    uint32_t reserved[32 - 7];
};

struct hw_device_t {
    uint32_t tag;
    uint32_t version;
    hw_module_t *module;
    uint32_t reserved[12];
    int (*close)(hw_device_t *);
};

typedef struct native_handle {
    int version;
    int numFds;
    int numInts;
    int data[0];
} native_handle_t;
typedef const native_handle_t *buffer_handle_t;

typedef struct alloc_device_t {
    struct hw_device_t common;
    int (*alloc)(struct alloc_device_t *, int w, int h, int format, int usage,
                 buffer_handle_t *, int *stride);
    int (*free)(struct alloc_device_t *, buffer_handle_t);
    void (*dump)(struct alloc_device_t *, char *, int);
    void *reserved_proc[7];
} alloc_device_t;

typedef struct framebuffer_device_t {
    struct hw_device_t common;
    const uint32_t flags;
    const uint32_t width;
    const uint32_t height;
    const int stride;
    const int format;
    const float xdpi;
    const float ydpi;
    const float fps;
    const int minSwapInterval;
    const int maxSwapInterval;
    const int numFramebuffers;
    int reserved[7];
    int (*setSwapInterval)(struct framebuffer_device_t *, int);
    int (*setUpdateRect)(struct framebuffer_device_t *, int, int, int, int);
    int (*post)(struct framebuffer_device_t *, buffer_handle_t);
    int (*compositionComplete)(struct framebuffer_device_t *);
    void (*dump)(struct framebuffer_device_t *, char *, int);
    int (*enableScreen)(struct framebuffer_device_t *, int);
    void *reserved_proc[6];
} framebuffer_device_t;

typedef struct gralloc_module_t {
    struct hw_module_t common;
    int (*registerBuffer)(const struct gralloc_module_t *, buffer_handle_t);
    int (*unregisterBuffer)(const struct gralloc_module_t *, buffer_handle_t);
    int (*lock)(const struct gralloc_module_t *, buffer_handle_t, int usage,
                int l, int t, int w, int h, void **vaddr);
    int (*unlock)(const struct gralloc_module_t *, buffer_handle_t);
    int (*perform)(const struct gralloc_module_t *, int operation, ...);
    void *rest[8];
} gralloc_module_t;

#define GRALLOC_USAGE_SW_READ_OFTEN   0x00000003
#define GRALLOC_USAGE_SW_WRITE_OFTEN  0x00000030
#define GRALLOC_USAGE_HW_FB           0x00001000
#define GRALLOC_USAGE_HW_COMPOSER     0x00000800
#define HAL_PIXEL_FORMAT_RGBA_8888    1
#define HAL_PIXEL_FORMAT_RGBX_8888    2
#define HAL_PIXEL_FORMAT_BGRA_8888    5

static const char *fmt_name(int f)
{
    switch (f) {
        case HAL_PIXEL_FORMAT_RGBA_8888: return "RGBA_8888";
        case HAL_PIXEL_FORMAT_RGBX_8888: return "RGBX_8888";
        case HAL_PIXEL_FORMAT_BGRA_8888: return "BGRA_8888";
        default: return "?";
    }
}

int main(int argc, char **argv)
{
    const char *so = argc > 1 ? argv[1] : "/vendor/lib/hw/gralloc.mt6580.so";

    void *h = dlopen(so, RTLD_NOW);
    if (!h) { printf("FAIL dlopen %s: %s\n", so, dlerror()); return 1; }
    printf("ok   dlopen %s\n", so);

    gralloc_module_t *mod = dlsym(h, "HMI");
    if (!mod) { printf("FAIL no HMI symbol: %s\n", dlerror()); return 1; }
    printf("ok   HMI  id=%s name=%s\n", mod->common.id ? mod->common.id : "?",
           mod->common.name ? mod->common.name : "?");
    printf("     module_api=0x%04x hal_api=0x%04x methods=%p\n",
           mod->common.module_api_version, mod->common.hal_api_version,
           (void *)mod->common.methods);
    if (!mod->common.methods || !mod->common.methods->open) {
        printf("FAIL module has no open()\n"); return 1;
    }

    hw_device_t *dev = NULL;
    int rc = mod->common.methods->open(&mod->common, "gpu0", &dev);
    printf("%s gpu0 (allocator) rc=%d dev=%p\n", rc == 0 ? "ok  " : "FAIL", rc, (void *)dev);
    alloc_device_t *alloc = (alloc_device_t *)dev;

    hw_device_t *fbdev = NULL;
    rc = mod->common.methods->open(&mod->common, "fb", &fbdev);
    if (rc != 0 || !fbdev) {
        printf("FAIL fb (framebuffer device) rc=%d - no fb device in this gralloc.\n", rc);
        printf("     => route A is not available; presentation needs hwcomposer.\n");
        return 2;
    }
    framebuffer_device_t *fb = (framebuffer_device_t *)fbdev;
    printf("ok   fb   %ux%u stride=%d format=%d(%s) fps=%.1f buffers=%d flags=0x%x\n",
           fb->width, fb->height, fb->stride, fb->format, fmt_name(fb->format),
           (double)fb->fps, fb->numFramebuffers, fb->flags);
    printf("     post=%p setSwapInterval=%p enableScreen=%p\n",
           (void *)fb->post, (void *)fb->setSwapInterval, (void *)fb->enableScreen);
    if (!fb->post) { printf("FAIL fb device exposes no post()\n"); return 2; }

    if (!alloc || !alloc->alloc) { printf("FAIL no allocator to make a buffer with\n"); return 1; }

    buffer_handle_t buf = NULL;
    int stride = 0;
    rc = alloc->alloc(alloc, (int)fb->width, (int)fb->height, fb->format,
                      GRALLOC_USAGE_HW_FB | GRALLOC_USAGE_SW_WRITE_OFTEN,
                      &buf, &stride);
    printf("%s alloc %ux%u rc=%d handle=%p stride=%d\n",
           rc == 0 ? "ok  " : "FAIL", fb->width, fb->height, rc, (void *)buf, stride);
    if (rc != 0 || !buf) return 1;

    void *vaddr = NULL;
    rc = mod->lock(mod, buf, GRALLOC_USAGE_SW_WRITE_OFTEN, 0, 0,
                   (int)fb->width, (int)fb->height, &vaddr);
    printf("%s lock rc=%d vaddr=%p\n", rc == 0 ? "ok  " : "FAIL", rc, vaddr);
    if (rc == 0 && vaddr) {
        /* Solid magenta: nothing else on this device draws that, so if the
         * panel turns magenta the post genuinely reached the glass. */
        uint32_t *p = vaddr;
        size_t n = (size_t)stride * fb->height;
        for (size_t i = 0; i < n; i++) p[i] = 0xFFFF00FF;
        mod->unlock(mod, buf);
    }

    rc = fb->post(fb, buf);
    printf("%s post rc=%d\n", rc == 0 ? "ok  " : "FAIL", rc);
    printf("\n  >>> LOOK AT THE PANEL. Magenta means route A works.\n");
    printf("      A zero rc with an unchanged screen is the stub case.\n");
    return rc == 0 ? 0 : 3;
}
