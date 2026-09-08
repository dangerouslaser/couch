/*
 * Copyright (C) 2017 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/* Android 32-bit gralloc HAL ABI; shared with the existing fbprobe. */
#pragma once
#include <stdint.h>
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


/* Layout from AOSP android-8.1.0_r1 nativebase.h (Apache-2.0).
 * frameworks/native/libs/nativebase/include/nativebase/nativebase.h
 * These offsets are ABI, not a private gralloc handle interpretation. */
struct native_base {
    int magic, version;
    void *reserved[4];
    void (*incRef)(struct native_base *);
    void (*decRef)(struct native_base *);
};
struct native_buffer {
    struct native_base common;
    int width, height, stride, format, usage_deprecated;
    uintptr_t layerCount;
    void *reserved[1];
    buffer_handle_t handle;
    uint64_t usage;
    void *reserved_proc[8 - sizeof(uint64_t) / sizeof(void *)];
};
_Static_assert(sizeof(void *) == 4, "HA100 graphics blobs use a 32-bit ABI");
_Static_assert(sizeof(struct native_buffer) == 96, "Android native buffer ABI");
