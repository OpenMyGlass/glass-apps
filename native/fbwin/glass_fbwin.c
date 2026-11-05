/* SPDX-License-Identifier: Apache-2.0
 * libglass_fbwin: an ANativeWindow over the gralloc framebuffer HAL, for a plain bionic
 * process with no SurfaceFlinger (Model B). Modelled on AOSP 4.4
 * frameworks/native/libs/ui/FramebufferNativeWindow.cpp (Apache-2.0).
 *
 * Exports the provider ABI of glass-gfx-gles' gles_probe:
 *   glass_gfx_create_native_window(u32 w, u32 h, i32 format) -> ANativeWindow*
 *   glass_gfx_native_window_format() -> i32
 * Single client, single thread: the renderer's. */
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <stdarg.h>
#include "android_abi.h"

#define MAX_BUFS 3

struct fbwin {
    struct glass_ANativeWindow win;
    struct framebuffer_device_t *fb;
    struct alloc_device_t *gr;
    struct ANativeWindowBuffer bufs[MAX_BUFS];
    int nbufs;
    int head;  /* next buffer to hand out */
    int nfree;
    int frames;
};

#define LOG(...) do { fprintf(stderr, "fbwin: " __VA_ARGS__); fputc('\n', stderr); } while (0)

static void noref(struct android_native_base_t *b) { (void)b; }

static int set_interval(struct glass_ANativeWindow *w, int interval)
{
    struct fbwin *f = (struct fbwin *)w;
    return f->fb->setSwapInterval ? f->fb->setSwapInterval(f->fb, interval) : 0;
}

static int dequeue(struct glass_ANativeWindow *w, struct ANativeWindowBuffer **out, int *fence)
{
    struct fbwin *f = (struct fbwin *)w;
    if (f->nfree <= 0) {
        LOG("dequeue with no free buffer");
        return -EBUSY;
    }
    *out = &f->bufs[f->head];
    f->head = (f->head + 1) % f->nbufs;
    f->nfree--;
    if (fence)
        *fence = -1;
    return 0;
}

static int queue(struct glass_ANativeWindow *w, struct ANativeWindowBuffer *b, int fence)
{
    struct fbwin *f = (struct fbwin *)w;
    int rc = f->fb->post(f->fb, b->handle);
    (void)fence;
    f->nfree++;
    f->frames++;
    if (rc)
        LOG("post failed: %d (frame %d)", rc, f->frames);
    return rc;
}

static int cancel(struct glass_ANativeWindow *w, struct ANativeWindowBuffer *b, int fence)
{
    struct fbwin *f = (struct fbwin *)w;
    (void)b; (void)fence;
    f->nfree++;
    return 0;
}

static int dequeue_old(struct glass_ANativeWindow *w, struct ANativeWindowBuffer **b)
{
    return dequeue(w, b, NULL);
}
static int lock_old(struct glass_ANativeWindow *w, struct ANativeWindowBuffer *b)
{
    (void)w; (void)b;
    return 0;
}
static int queue_old(struct glass_ANativeWindow *w, struct ANativeWindowBuffer *b)
{
    return queue(w, b, -1);
}
static int cancel_old(struct glass_ANativeWindow *w, struct ANativeWindowBuffer *b)
{
    return cancel(w, b, -1);
}

static int query(const struct glass_ANativeWindow *w, int what, int *v)
{
    const struct fbwin *f = (const struct fbwin *)w;
    switch (what) {
    case NATIVE_WINDOW_WIDTH:
    case NATIVE_WINDOW_DEFAULT_WIDTH:
        *v = (int)f->fb->width;
        return 0;
    case NATIVE_WINDOW_HEIGHT:
    case NATIVE_WINDOW_DEFAULT_HEIGHT:
        *v = (int)f->fb->height;
        return 0;
    case NATIVE_WINDOW_FORMAT:
        *v = f->fb->format;
        return 0;
    case NATIVE_WINDOW_CONCRETE_TYPE:
        *v = NATIVE_WINDOW_FRAMEBUFFER;
        return 0;
    case NATIVE_WINDOW_CONSUMER_USAGE_BITS:
        *v = GRALLOC_USAGE_HW_FB | GRALLOC_USAGE_HW_RENDER;
        return 0;
    case NATIVE_WINDOW_MIN_UNDEQUEUED_BUFFERS:
    case NATIVE_WINDOW_QUEUES_TO_WINDOW_COMPOSER:
    case NATIVE_WINDOW_TRANSFORM_HINT:
    case NATIVE_WINDOW_CONSUMER_RUNNING_BEHIND:
        *v = 0;
        return 0;
    }
    *v = 0;
    LOG("query %d: unknown", what);
    return -EINVAL;
}

static int perform(struct glass_ANativeWindow *w, int op, ...)
{
    (void)w; (void)op;
    return 0; /* usage, connect, geometry: the framebuffer's are fixed */
}

static struct fbwin *the_window;
static int tried;
typedef int (*hw_get_module_fn)(const char *, const struct hw_module_t **);

static struct fbwin *open_window(void)
{
    const struct hw_module_t *mod = NULL;
    struct hw_device_t *dev = NULL;
    struct fbwin *f;
    int rc, i;
    void *hw = dlopen("libhardware.so", RTLD_NOW);
    hw_get_module_fn get = hw ? (hw_get_module_fn)dlsym(hw, "hw_get_module") : NULL;

    if (the_window || tried)
        return the_window;
    tried = 1;
    if (!get) {
        LOG("libhardware: %s", dlerror());
        return NULL;
    }
    rc = get("gralloc", &mod);
    if (rc) {
        LOG("hw_get_module(gralloc): %d", rc);
        return NULL;
    }
    LOG("gralloc module: id=%s name=%s author=%s", mod->id, mod->name, mod->author);
    f = calloc(1, sizeof(*f));
    if (!f)
        return NULL;
    rc = mod->methods->open(mod, "fb0", &dev);
    if (rc) {
        LOG("framebuffer open: %d", rc);
        free(f);
        return NULL;
    }
    f->fb = (struct framebuffer_device_t *)dev;
    rc = mod->methods->open(mod, "gpu0", &dev);
    if (rc) {
        LOG("gralloc open: %d", rc);
        free(f);
        return NULL;
    }
    f->gr = (struct alloc_device_t *)dev;
    LOG("fb: %ux%u stride=%d format=%d fps=%.1f swap=%d..%d numFramebuffers=%d",
        f->fb->width, f->fb->height, f->fb->stride, f->fb->format, (double)f->fb->fps,
        f->fb->minSwapInterval, f->fb->maxSwapInterval, f->fb->numFramebuffers);
    f->nbufs = f->fb->numFramebuffers;
    if (f->nbufs > MAX_BUFS)
        f->nbufs = MAX_BUFS;
    if (f->nbufs < 1)
        f->nbufs = MAX_BUFS; /* the IMG HAL reports 0; its kernel side keeps 3 TILER buffers */
    for (i = 0; i < f->nbufs; i++) {
        struct ANativeWindowBuffer *b = &f->bufs[i];
        int stride = 0;
        b->common.magic = ANDROID_NATIVE_BUFFER_MAGIC;
        b->common.version = sizeof(*b);
        b->common.incRef = noref;
        b->common.decRef = noref;
        b->width = (int)f->fb->width;
        b->height = (int)f->fb->height;
        b->format = f->fb->format;
        b->usage = GRALLOC_USAGE_HW_FB;
        rc = f->gr->alloc(f->gr, b->width, b->height, b->format, b->usage, &b->handle, &stride);
        if (rc) {
            LOG("alloc buffer %d: %d", i, rc);
            if (i == 0) {
                free(f);
                return NULL;
            }
            f->nbufs = i;
            break;
        }
        b->stride = stride;
        LOG("buffer %d: handle=%p stride=%d", i, b->handle, stride);
    }
    f->nfree = f->nbufs;
    f->win.common.magic = ANDROID_NATIVE_WINDOW_MAGIC;
    f->win.common.version = sizeof(f->win);
    f->win.common.incRef = noref;
    f->win.common.decRef = noref;
    f->win.minSwapInterval = f->fb->minSwapInterval;
    f->win.maxSwapInterval = f->fb->maxSwapInterval;
    f->win.xdpi = f->fb->xdpi;
    f->win.ydpi = f->fb->ydpi;
    f->win.setSwapInterval = set_interval;
    f->win.dequeueBuffer_DEPRECATED = dequeue_old;
    f->win.lockBuffer_DEPRECATED = lock_old;
    f->win.queueBuffer_DEPRECATED = queue_old;
    f->win.query = query;
    f->win.perform = perform;
    f->win.cancelBuffer_DEPRECATED = cancel_old;
    f->win.dequeueBuffer = dequeue;
    f->win.queueBuffer = queue;
    f->win.cancelBuffer = cancel;
    the_window = f;
    return f;
}

void *glass_gfx_create_native_window(uint32_t w, uint32_t h, int32_t format)
{
    struct fbwin *f = open_window();
    if (!f)
        return NULL;
    if (w != f->fb->width || h != f->fb->height || (format && format != f->fb->format))
        LOG("asked for %ux%u format %d, the framebuffer's are used", w, h, format);
    return &f->win;
}

int32_t glass_gfx_native_window_format(void)
{
    struct fbwin *f = open_window();
    return f ? f->fb->format : -1;
}
