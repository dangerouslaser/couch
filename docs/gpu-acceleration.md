# GPU acceleration on the HA100

The panel work that hurts is full-screen image compositing: a crossfade between
two 480x800 backdrops costs **44-57ms per frame** on the CPU, in both LVGL and
Slint. Everything else fits comfortably in a 16ms budget. So this document is
about that one workload.

## What the hardware has

| piece | state |
|---|---|
| Mali-400 MP1 | present; kernel driver built in (`/dev/mali`, 854 symbols in kallsyms) |
| ION allocator | `/dev/ion` present |
| binder | `/dev/binder`, `/dev/hwbinder` present |
| DRM/KMS | **absent** - no `/dev/dri`, so Mesa/Lima is not an option |

Lima (the open Mali-400 driver in Mesa) needs DRM/KMS. This kernel has none, and
rebuilding it is blocked - see the notes on the vendor tree. That leaves the
proprietary blobs, which means libhybris.

## What the stock firmware ships

From the device's own `vendor` (p14) and `system` (p21):

    /vendor/lib/hw/gralloc.mt6580.so        28,928
    /vendor/lib/hw/hwcomposer.mt6580.so    386,532
    /vendor/lib/hw/memtrack.mt6580.so       16,296
    /vendor/lib/egl/libGLES_mali.so      1,323,632
    /system/lib/egl/libGLES_android.so     108,452   (software fallback)

The dependency split is what decides the shape of any port:

**gralloc** needs only `libc++ libc libcutils libdl libged libgralloc_extra
libhardware libion libion_mtk liblog libm libsec_mem libutils`. No binder, no
HIDL, no libgui. All present on the device.

**hwcomposer** additionally needs `libbinder libhwbinder libhidlbase
libhidltransport libgui libui libui_ext libpower libm4u libbwc libpq_prot
libdpframework libsync`. That is most of SurfaceFlinger's supporting cast.

**libGLES_mali** needs `libcutils libgralloc_extra libhardware libged libgpu_aux
libdpframework libmtk_drvb liblog libutils` - notably *no* binder or HIDL, so the
GL driver itself does not drag in the Android graphics service stack.

## Two possible routes

    Slint -> GLES2 -> libhybris -> gralloc + libGLES_mali -> kernel  (route A)
    Slint -> GLES2 -> libhybris -> gralloc + HWC + libGLES_mali      (route B)

**Route A (no HWC)** is far smaller: allocate buffers with gralloc, render with
Mali, present by posting to the framebuffer. Its risk is concentrated in one
place - `gralloc.mt6580.so` contains `fb_post`, but also the string:

    "POST a ION_FB_HEAP buffer, not implementation yet. Show the 1st frame instead."

If that path is a stub in this build, presentation has to come from somewhere
else: either blit the rendered GL buffer back to `/dev/fb0` by hand (losing some
of the win, but not all - the compositing would still be on the GPU), or fall
through to route B.

**Route B (with HWC)** is the Halium/Ubuntu Touch shape and is known to work on
MediaTek parts, but it means bringing up binder, HIDL and enough of libgui for
`HWComposerNativeWindow`. Much larger surface area.

## Unknowns to resolve before committing

1. Is `fb_post` in this gralloc actually functional, or a stub? Decides A vs B.
2. Does `libGLES_mali` contain a non-Android EGL platform, or only ANativeWindow?
   Its `__egl_platform_*` symbols carry `_android` variants; a generic one is not
   confirmed.
3. Does libhybris build against musl? It is normally built for glibc, Alpine does
   not package it, and it ships its own linker - so this is a real porting task,
   not a package install.
4. Slint's `FemtoVGRenderer` needs an EGL context supplied through a custom
   OpenGL context wrapper. Documented for embedded, but another integration.

## The cheaper alternative, first

NEON is unexploited and this is a Cortex-A7. LVGL ships NEON blenders, but
`lv_blend_neon.S` is written in GNU `as` macro syntax that LLVM's integrated
assembler rejects, and the NDK is LLVM-only. At roughly 130ns per composited
pixel we are plainly not using SIMD at all, so hand-porting those kernels (or
writing intrinsics) is a day of work against libhybris's weeks, and a 3-4x blend
speedup would bring the 50ms crossfade close to budget on its own.

Worth trying before committing to the Android graphics stack.
