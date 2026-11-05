/* SPDX-License-Identifier: Apache-2.0
 * The few Android 4.4 HAL structs the framebuffer window needs. The NDK does not ship
 * hardware/gralloc.h, hardware/fb.h or system/window.h. Layouts follow AOSP android-4.4.4
 * (Apache-2.0); the stock blobs are built against them. */
#ifndef GLASS_ANDROID_ABI_H
#define GLASS_ANDROID_ABI_H
#include <stdint.h>
#include <stddef.h>

typedef const void *buffer_handle_t;

struct android_native_base_t {
    int magic;
    int version;
    void *reserved[4];
    void (*incRef)(struct android_native_base_t *);
    void (*decRef)(struct android_native_base_t *);
};

#define ANDROID_MAGIC(a, b, c, d) \
    (((unsigned)(a) << 24) | ((unsigned)(b) << 16) | ((unsigned)(c) << 8) | (unsigned)(d))
#define ANDROID_NATIVE_WINDOW_MAGIC ANDROID_MAGIC('_', 'w', 'n', 'd')
#define ANDROID_NATIVE_BUFFER_MAGIC ANDROID_MAGIC('_', 'b', 'f', 'r')

struct ANativeWindowBuffer {
    struct android_native_base_t common;
    int width;
    int height;
    int stride;
    int format;
    int usage;
    void *reserved[2];
    buffer_handle_t handle;
    void *reserved_proc[8];
};

struct glass_ANativeWindow {
    struct android_native_base_t common;
    int minSwapInterval;
    int maxSwapInterval;
    float xdpi;
    float ydpi;
    intptr_t oem[4];
    /* XE24's libEGL calls setSwapInterval at offset 68 and perform at 88: one word more than
     * the AOSP 4.4 header gives (measured, 2026-11-05). */
    intptr_t glass_xe24_pad;
    int (*setSwapInterval)(struct glass_ANativeWindow *, int);
    int (*dequeueBuffer_DEPRECATED)(struct glass_ANativeWindow *, struct ANativeWindowBuffer **);
    int (*lockBuffer_DEPRECATED)(struct glass_ANativeWindow *, struct ANativeWindowBuffer *);
    int (*queueBuffer_DEPRECATED)(struct glass_ANativeWindow *, struct ANativeWindowBuffer *);
    int (*query)(const struct glass_ANativeWindow *, int what, int *value);
    int (*perform)(struct glass_ANativeWindow *, int op, ...);
    int (*cancelBuffer_DEPRECATED)(struct glass_ANativeWindow *, struct ANativeWindowBuffer *);
    int (*dequeueBuffer)(struct glass_ANativeWindow *, struct ANativeWindowBuffer **, int *fence);
    int (*queueBuffer)(struct glass_ANativeWindow *, struct ANativeWindowBuffer *, int fence);
    int (*cancelBuffer)(struct glass_ANativeWindow *, struct ANativeWindowBuffer *, int fence);
};

enum {
    NATIVE_WINDOW_WIDTH = 0,
    NATIVE_WINDOW_HEIGHT = 1,
    NATIVE_WINDOW_FORMAT = 2,
    NATIVE_WINDOW_MIN_UNDEQUEUED_BUFFERS = 3,
    NATIVE_WINDOW_QUEUES_TO_WINDOW_COMPOSER = 4,
    NATIVE_WINDOW_CONCRETE_TYPE = 5,
    NATIVE_WINDOW_DEFAULT_WIDTH = 6,
    NATIVE_WINDOW_DEFAULT_HEIGHT = 7,
    NATIVE_WINDOW_TRANSFORM_HINT = 8,
    NATIVE_WINDOW_CONSUMER_RUNNING_BEHIND = 9,
    NATIVE_WINDOW_CONSUMER_USAGE_BITS = 10,
};
#define NATIVE_WINDOW_FRAMEBUFFER 0

struct hw_module_t;
struct hw_device_t;
struct hw_module_methods_t {
    int (*open)(const struct hw_module_t *module, const char *id, struct hw_device_t **device);
};
struct hw_module_t {
    uint32_t tag;
    uint16_t module_api_version;
    uint16_t hal_api_version;
    const char *id;
    const char *name;
    const char *author;
    struct hw_module_methods_t *methods;
    void *dso;
    uint32_t reserved[32 - 7];
};
struct hw_device_t {
    uint32_t tag;
    uint32_t version;
    struct hw_module_t *module;
    uint32_t reserved[12];
    int (*close)(struct hw_device_t *device);
};

struct alloc_device_t {
    struct hw_device_t common;
    int (*alloc)(struct alloc_device_t *dev, int w, int h, int format, int usage,
                 buffer_handle_t *handle, int *stride);
    int (*free)(struct alloc_device_t *dev, buffer_handle_t handle);
    void (*dump)(struct alloc_device_t *dev, char *buff, int buff_len);
    void *reserved_proc[7];
};

struct framebuffer_device_t {
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
    int (*setSwapInterval)(struct framebuffer_device_t *window, int interval);
    int (*setUpdateRect)(struct framebuffer_device_t *window, int l, int t, int w, int h);
    int (*post)(struct framebuffer_device_t *dev, buffer_handle_t buffer);
    int (*compositionComplete)(struct framebuffer_device_t *dev);
    void (*dump)(struct framebuffer_device_t *dev, char *buff, int buff_len);
    int (*enableScreen)(struct framebuffer_device_t *dev, int enable);
    void *reserved_proc[6];
};

#define GRALLOC_USAGE_HW_TEXTURE 0x00000100
#define GRALLOC_USAGE_HW_RENDER 0x00000200
#define GRALLOC_USAGE_HW_FB 0x00001000

#endif
