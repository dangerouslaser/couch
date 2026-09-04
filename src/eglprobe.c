/*
 * Can we get a GLES2 context on this device with no window system?
 *
 * The gralloc probe showed there is no framebuffer HAL, so presentation has to
 * be either hwcomposer (binder + HIDL + libgui) or "composite on the GPU, copy
 * the result to /dev/fb0 ourselves" - which is only worth pursuing if EGL will
 * come up headless. That is what this answers.
 *
 * libGLES_mali is dlopened directly and its EGL/GLES entry points looked up by
 * name, rather than going through Android's libEGL loader: the loader reads
 * egl.cfg and system properties and adds a layer of things that can fail for
 * reasons that have nothing to do with the driver.
 *
 * Rendering goes to an FBO, and the result is read back with glReadPixels. On a
 * tiled GPU readback is the expensive part, so it is timed - if it is slow, the
 * copy-back plan needs the zero-copy EGLImage path instead, and if it is fast
 * enough the simple version is worth having first.
 */
#include <dlfcn.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <time.h>

typedef void *EGLDisplay, *EGLConfig, *EGLContext, *EGLSurface;
typedef unsigned EGLenum, EGLBoolean;
typedef int EGLint;

#define EGL_DEFAULT_DISPLAY      ((void *)0)
#define EGL_NO_CONTEXT           ((void *)0)
#define EGL_NO_SURFACE           ((void *)0)
#define EGL_VENDOR               0x3053
#define EGL_VERSION_STR          0x3054
#define EGL_EXTENSIONS           0x3055
#define EGL_CLIENT_APIS          0x308D
#define EGL_SURFACE_TYPE         0x3033
#define EGL_PBUFFER_BIT          0x0001
#define EGL_RENDERABLE_TYPE      0x3040
#define EGL_OPENGL_ES2_BIT       0x0004
#define EGL_ALPHA_SIZE           0x3021
#define EGL_BLUE_SIZE            0x3022
#define EGL_GREEN_SIZE           0x3023
#define EGL_RED_SIZE             0x3024
#define EGL_NONE                 0x3038
#define EGL_HEIGHT               0x3056
#define EGL_WIDTH                0x3057
#define EGL_CONTEXT_CLIENT_VERSION 0x3098
#define EGL_OPENGL_ES_API        0x30A0

#define GL_VENDOR                0x1F00
#define GL_RENDERER              0x1F01
#define GL_VERSION_STR           0x1F02
#define GL_EXTENSIONS            0x1F03
#define GL_TEXTURE_2D            0x0DE1
#define GL_RGBA                  0x1908
#define GL_UNSIGNED_BYTE         0x1401
#define GL_TEXTURE_MIN_FILTER    0x2801
#define GL_TEXTURE_MAG_FILTER    0x2800
#define GL_LINEAR                0x2601
#define GL_FRAMEBUFFER           0x8D40
#define GL_COLOR_ATTACHMENT0     0x8CE0
#define GL_FRAMEBUFFER_COMPLETE  0x8CD5
#define GL_COLOR_BUFFER_BIT      0x00004000

#define W 480
#define H 800

static void *lib;
#define SYM(name) do { \
    *(void **)(&name) = dlsym(lib, #name); \
    if (!name) { printf("FAIL missing symbol %s\n", #name); return 1; } \
} while (0)

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

int main(int argc, char **argv)
{
    const char *so = argc > 1 ? argv[1] : "/vendor/lib/egl/libGLES_mali.so";

    lib = dlopen(so, RTLD_NOW | RTLD_GLOBAL);
    if (!lib) { printf("FAIL dlopen %s: %s\n", so, dlerror()); return 1; }
    printf("ok   dlopen %s\n", so);

    EGLDisplay (*eglGetDisplay)(void *);
    EGLBoolean (*eglInitialize)(EGLDisplay, EGLint *, EGLint *);
    const char *(*eglQueryString)(EGLDisplay, EGLint);
    EGLBoolean (*eglBindAPI)(EGLenum);
    EGLBoolean (*eglChooseConfig)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *);
    EGLContext (*eglCreateContext)(EGLDisplay, EGLConfig, EGLContext, const EGLint *);
    EGLSurface (*eglCreatePbufferSurface)(EGLDisplay, EGLConfig, const EGLint *);
    EGLBoolean (*eglMakeCurrent)(EGLDisplay, EGLSurface, EGLSurface, EGLContext);
    EGLint (*eglGetError)(void);

    SYM(eglGetDisplay); SYM(eglInitialize); SYM(eglQueryString); SYM(eglBindAPI);
    SYM(eglChooseConfig); SYM(eglCreateContext); SYM(eglCreatePbufferSurface);
    SYM(eglMakeCurrent); SYM(eglGetError);

    EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    printf("%s eglGetDisplay(EGL_DEFAULT_DISPLAY) -> %p err=0x%04x\n",
           dpy ? "ok  " : "FAIL", dpy, eglGetError());
    if (!dpy) {
        /* Some Mali builds only accept a display they recognise. Try the
         * handful of values a platform layer might be looking for before
         * concluding the driver will not come up. */
        for (uintptr_t cand = 1; cand <= 2 && !dpy; cand++) {
            dpy = eglGetDisplay((void *)cand);
            printf("     eglGetDisplay(%lu) -> %p err=0x%04x\n",
                   (unsigned long)cand, dpy, eglGetError());
        }
    }
    if (!dpy) return 2;

    EGLint major = 0, minor = 0;
    if (!eglInitialize(dpy, &major, &minor)) {
        printf("FAIL eglInitialize err=0x%04x - EGL will not come up headless\n", eglGetError());
        return 2;
    }
    printf("ok   eglInitialize %d.%d\n", major, minor);
    printf("     vendor : %s\n", eglQueryString(dpy, EGL_VENDOR));
    printf("     version: %s\n", eglQueryString(dpy, EGL_VERSION_STR));
    printf("     apis   : %s\n", eglQueryString(dpy, EGL_CLIENT_APIS));
    const char *exts = eglQueryString(dpy, EGL_EXTENSIONS);
    printf("     image_native_buffer: %s\n",
           exts && strstr(exts, "EGL_ANDROID_image_native_buffer") ? "yes" : "NO");
    printf("     surfaceless       : %s\n",
           exts && strstr(exts, "EGL_KHR_surfaceless_context") ? "yes" : "no");

    eglBindAPI(EGL_OPENGL_ES_API);

    const EGLint cfg_attrs[] = {
        EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
        EGL_NONE
    };
    EGLConfig cfg; EGLint ncfg = 0;
    if (!eglChooseConfig(dpy, cfg_attrs, &cfg, 1, &ncfg) || ncfg < 1) {
        printf("FAIL eglChooseConfig err=0x%04x (no pbuffer-capable ES2 config)\n", eglGetError());
        return 2;
    }
    printf("ok   eglChooseConfig (%d config%s)\n", ncfg, ncfg == 1 ? "" : "s");

    const EGLint pb_attrs[] = { EGL_WIDTH, W, EGL_HEIGHT, H, EGL_NONE };
    EGLSurface surf = eglCreatePbufferSurface(dpy, cfg, pb_attrs);
    printf("%s eglCreatePbufferSurface %dx%d -> %p err=0x%04x\n",
           surf ? "ok  " : "FAIL", W, H, surf, eglGetError());
    if (!surf) return 2;

    const EGLint ctx_attrs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
    EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attrs);
    printf("%s eglCreateContext -> %p err=0x%04x\n", ctx ? "ok  " : "FAIL", ctx, eglGetError());
    if (!ctx) return 2;

    if (!eglMakeCurrent(dpy, surf, surf, ctx)) {
        printf("FAIL eglMakeCurrent err=0x%04x\n", eglGetError());
        return 2;
    }
    printf("ok   eglMakeCurrent\n");

    const char *(*glGetString)(unsigned);
    void (*glClearColor)(float, float, float, float);
    void (*glClear)(unsigned);
    void (*glFinish)(void);
    void (*glReadPixels)(int, int, int, int, unsigned, unsigned, void *);
    void (*glGenTextures)(int, unsigned *);
    void (*glBindTexture)(unsigned, unsigned);
    void (*glTexImage2D)(unsigned, int, int, int, int, int, unsigned, unsigned, const void *);
    void (*glTexParameteri)(unsigned, unsigned, int);
    void (*glGenFramebuffers)(int, unsigned *);
    void (*glBindFramebuffer)(unsigned, unsigned);
    void (*glFramebufferTexture2D)(unsigned, unsigned, unsigned, unsigned, int);
    unsigned (*glCheckFramebufferStatus)(unsigned);
    void (*glViewport)(int, int, int, int);

    SYM(glGetString); SYM(glClearColor); SYM(glClear); SYM(glFinish);
    SYM(glReadPixels); SYM(glGenTextures); SYM(glBindTexture); SYM(glTexImage2D);
    SYM(glTexParameteri); SYM(glGenFramebuffers); SYM(glBindFramebuffer);
    SYM(glFramebufferTexture2D); SYM(glCheckFramebufferStatus); SYM(glViewport);

    printf("     GL_VENDOR  : %s\n", glGetString(GL_VENDOR));
    printf("     GL_RENDERER: %s\n", glGetString(GL_RENDERER));
    printf("     GL_VERSION : %s\n", glGetString(GL_VERSION_STR));

    unsigned tex = 0, fbo = 0;
    glGenTextures(1, &tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
    glGenFramebuffers(1, &fbo);
    glBindFramebuffer(GL_FRAMEBUFFER, fbo);
    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    unsigned st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
    printf("%s FBO %dx%d status=0x%04x%s\n", st == GL_FRAMEBUFFER_COMPLETE ? "ok  " : "FAIL",
           W, H, st, st == GL_FRAMEBUFFER_COMPLETE ? " (complete)" : "");
    if (st != GL_FRAMEBUFFER_COMPLETE) return 3;

    glViewport(0, 0, W, H);

    static uint8_t pixels[W * H * 4];
    const int iters = 30;

    glClearColor(1.0f, 0.0f, 1.0f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    glFinish();

    double t = now_s();
    for (int i = 0; i < iters; i++) {
        glClearColor((i & 1) ? 1.0f : 0.0f, 0.25f, 0.75f, 1.0f);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    glFinish();
    printf("     full-screen clear      : %6.2f ms/frame\n", (now_s() - t) * 1000.0 / iters);

    t = now_s();
    for (int i = 0; i < iters; i++)
        glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, pixels);
    glFinish();
    printf("     glReadPixels %dx%d     : %6.2f ms/frame\n", W, H,
           (now_s() - t) * 1000.0 / iters);

    printf("     first pixel = %02x %02x %02x %02x (expect the last clear colour)\n",
           pixels[0], pixels[1], pixels[2], pixels[3]);
    printf("\n  GPU is usable headless. Readback cost decides copy-back vs EGLImage.\n");
    return 0;
}
