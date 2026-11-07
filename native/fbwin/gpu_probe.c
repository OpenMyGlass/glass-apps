/* SPDX-License-Identifier: Apache-2.0
 * gpu_probe: smallest GLES2 experiments on the SGX540 under the stock bionic linker.
 *   gpu_probe pbuffer            EGL up, shader, draw on a pbuffer, glReadPixels
 *   gpu_probe window N [interval]  N frames through libglass_fbwin, timing, fb read-back
 * All lines go to stdout, prefixed by the init. */
#include <EGL/egl.h>
#include <GLES2/gl2.h>
#include <dlfcn.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <linux/fb.h>

#include <signal.h>
#include <ucontext.h>

static void on_crash(int sig, siginfo_t *si, void *uc_)
{
    ucontext_t *uc = uc_;
    unsigned long *sp = (unsigned long *)uc->uc_mcontext.arm_sp;
    char line[256];
    int i;
    FILE *m;
    printf("CRASH sig=%d addr=%p pc=%08lx lr=%08lx r0=%08lx r1=%08lx r2=%08lx r3=%08lx\n", sig,
           si->si_addr, uc->uc_mcontext.arm_pc, uc->uc_mcontext.arm_lr, uc->uc_mcontext.arm_r0,
           uc->uc_mcontext.arm_r1, uc->uc_mcontext.arm_r2, uc->uc_mcontext.arm_r3);
    for (i = 0; i < 64; i += 8)
        printf("stack+%02x: %08lx %08lx %08lx %08lx %08lx %08lx %08lx %08lx\n", i * 4, sp[i], sp[i + 1],
               sp[i + 2], sp[i + 3], sp[i + 4], sp[i + 5], sp[i + 6], sp[i + 7]);
    m = fopen("/proc/self/maps", "r");
    while (m && fgets(line, sizeof line, m))
        if (strstr(line, "r-xp"))
            printf("map %s", line);
    fflush(stdout);
    _exit(139);
}

#define W 640
#define H 360

static double now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000.0 + t.tv_nsec / 1e6;
}
static double cpu_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &t);
    return t.tv_sec * 1000.0 + t.tv_nsec / 1e6;
}

static void dump_configs(EGLDisplay d)
{
    EGLConfig c[64];
    EGLint n = 0, i;
    eglGetConfigs(d, c, 64, &n);
    printf("configs: %d\n", n);
    for (i = 0; i < n; i++) {
        EGLint r, g, b, a, st, rt, v, id, dp, sn;
        eglGetConfigAttrib(d, c[i], EGL_CONFIG_ID, &id);
        eglGetConfigAttrib(d, c[i], EGL_RED_SIZE, &r);
        eglGetConfigAttrib(d, c[i], EGL_GREEN_SIZE, &g);
        eglGetConfigAttrib(d, c[i], EGL_BLUE_SIZE, &b);
        eglGetConfigAttrib(d, c[i], EGL_ALPHA_SIZE, &a);
        eglGetConfigAttrib(d, c[i], EGL_DEPTH_SIZE, &dp);
        eglGetConfigAttrib(d, c[i], EGL_STENCIL_SIZE, &sn);
        eglGetConfigAttrib(d, c[i], EGL_SURFACE_TYPE, &st);
        eglGetConfigAttrib(d, c[i], EGL_RENDERABLE_TYPE, &rt);
        eglGetConfigAttrib(d, c[i], EGL_NATIVE_VISUAL_ID, &v);
        printf("cfg id=%d rgba=%d%d%d%d d=%d s=%d surf=0x%x rend=0x%x visual=%d\n", id, r, g, b, a,
               dp, sn, st, rt, v);
    }
}

static const char *VS = "#version 100\nattribute vec2 p;\nvoid main(){gl_Position=vec4(p,0.0,1.0);}\n";
static const char *FS =
    "#version 100\nprecision mediump float;\nuniform vec4 c;\nvoid main(){gl_FragColor=c;}\n";

static GLuint build(void)
{
    GLuint v = glCreateShader(GL_VERTEX_SHADER), f = glCreateShader(GL_FRAGMENT_SHADER), p;
    GLint ok = 0;
    double t0 = now_ms();
    char log[512];
    glShaderSource(v, 1, &VS, NULL);
    glCompileShader(v);
    glGetShaderiv(v, GL_COMPILE_STATUS, &ok);
    glGetShaderInfoLog(v, sizeof log, NULL, log);
    printf("vs compile=%d %s\n", ok, log);
    glShaderSource(f, 1, &FS, NULL);
    glCompileShader(f);
    glGetShaderiv(f, GL_COMPILE_STATUS, &ok);
    glGetShaderInfoLog(f, sizeof log, NULL, log);
    printf("fs compile=%d %s\n", ok, log);
    p = glCreateProgram();
    glAttachShader(p, v);
    glAttachShader(p, f);
    glBindAttribLocation(p, 0, "p");
    glLinkProgram(p);
    glGetProgramiv(p, GL_LINK_STATUS, &ok);
    printf("link=%d in %.1f ms\n", ok, now_ms() - t0);
    glUseProgram(p);
    return p;
}

static void rect(GLint uc, float x0, float y0, float x1, float y1, float r, float g, float b)
{
    /* clip space from top-left pixel coordinates, y down when flip */
    float q[8];
    q[0] = x0; q[1] = y0; q[2] = x1; q[3] = y0; q[4] = x0; q[5] = y1; q[6] = x1; q[7] = y1;
    glUniform4f(uc, r, g, b, 1.0f);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, q);
    glEnableVertexAttribArray(0);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
}

/* Quadrants: top-left red, top-right green, bottom-left blue, bottom-right white;
 * a magenta bar moves across the middle. */
static void scene(GLint uc, int frame)
{
    float bx = -1.0f + (float)(frame % 120) / 60.0f;
    glClearColor(0, 0, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    rect(uc, -1, 0, 0, 1, 1, 0, 0);
    rect(uc, 0, 0, 1, 1, 0, 1, 0);
    rect(uc, -1, -1, 0, 0, 0, 0, 1);
    rect(uc, 0, -1, 1, 0, 1, 1, 1);
    rect(uc, bx, -0.1f, bx + 0.1f, 0.1f, 1, 0, 1);
}

static unsigned fnv(const unsigned char *p, size_t n)
{
    unsigned h = 2166136261u;
    size_t i;
    for (i = 0; i < n; i++) {
        h ^= p[i];
        h *= 16777619u;
    }
    return h;
}

static void readback(const char *tag)
{
    static unsigned char px[W * H * 4];
    double t0 = now_ms();
    glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, px);
    printf("%s readpixels %.1f ms err=0x%x fnv=%08x gl(160,270 from bottom)=%02x%02x%02x%02x "
           "gl(480,270)=%02x%02x%02x%02x gl(160,90)=%02x%02x%02x%02x gl(480,90)=%02x%02x%02x%02x\n",
           tag, now_ms() - t0, glGetError(), fnv(px, sizeof px),
#define P(x, y) px[((y)*W + (x)) * 4], px[((y)*W + (x)) * 4 + 1], px[((y)*W + (x)) * 4 + 2], px[((y)*W + (x)) * 4 + 3]
           P(160, 270), P(480, 270), P(160, 90), P(480, 90));
}

static void fb_report(const char *tag)
{
    struct fb_var_screeninfo v;
    struct fb_fix_screeninfo x;
    int fd = open("/dev/fb0", O_RDONLY), b;
    unsigned char *m;
    if (fd < 0) {
        perror("fb0");
        return;
    }
    if (ioctl(fd, FBIOGET_VSCREENINFO, &v) || ioctl(fd, FBIOGET_FSCREENINFO, &x)) {
        perror("fb0 ioctl");
        close(fd);
        return;
    }
    printf("%s fb: %ux%u virt %ux%u off %u,%u bpp=%u line=%u smem=%u@0x%lx r@%u g@%u b@%u\n", tag,
           v.xres, v.yres, v.xres_virtual, v.yres_virtual, v.xoffset, v.yoffset, v.bits_per_pixel,
           x.line_length, x.smem_len, (unsigned long)x.smem_start, v.red.offset, v.green.offset,
           v.blue.offset);
    m = mmap(NULL, x.smem_len, PROT_READ, MAP_SHARED, fd, 0);
    if (m == MAP_FAILED) {
        perror("mmap fb");
        close(fd);
        return;
    }
    for (b = 0; b * x.line_length * v.yres < x.smem_len && b < 4; b++) {
        unsigned char *s = m + b * x.line_length * v.yres;
#define Q(xx, yy) *(unsigned *)(s + (yy) * x.line_length + (xx) * 4)
        if ((b + 1) * x.line_length * v.yres > x.smem_len)
            break;
        printf("%s fb buf%d: TL=%08x TR=%08x BL=%08x BR=%08x fnv=%08x\n", tag, b, Q(160, 90),
               Q(480, 90), Q(160, 270), Q(480, 270), fnv(s, x.line_length * v.yres));
    }
    munmap(m, x.smem_len);
    close(fd);
}

/* Reads the scanned-out buffer through /dev/mem: base and pitch as DISPC reports them. */
static int scan(unsigned long ba, unsigned long pitch, const char *dump)
{
    int fd = open("/dev/mem", O_RDONLY | O_SYNC);
    size_t len = pitch * H;
    unsigned long page = ba & ~0xfffUL;
    unsigned char *m;
    if (fd < 0) {
        perror("/dev/mem");
        return 1;
    }
    m = mmap(NULL, len + (ba - page), PROT_READ, MAP_SHARED, fd, page);
    if (m == MAP_FAILED) {
        perror("mmap /dev/mem");
        return 1;
    }
    m += ba - page;
#define S(xx, yy) *(unsigned *)(m + (yy) * pitch + (xx) * 4)
    printf("scan ba=%08lx pitch=%lu TL=%08x TR=%08x BL=%08x BR=%08x mid=%08x\n", ba, pitch, S(160, 90),
           S(480, 90), S(160, 270), S(480, 270), S(320, 180));
    if (dump) {
        FILE *f = fopen(dump, "w");
        int y;
        for (y = 0; f && y < H; y++)
            fwrite(m + y * pitch, 4, W, f);
        if (f)
            fclose(f);
    }
    return 0;
}

static int cmp(const void *a, const void *b)
{
    double d = *(const double *)a - *(const double *)b;
    return d < 0 ? -1 : d > 0;
}

static int run(int window, int frames, int interval)
{
    EGLDisplay d = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    EGLint maj = 0, min = 0, n = 0, vis = 0;
    EGLConfig cfg;
    EGLContext ctx;
    EGLSurface s;
    EGLint ca[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
    EGLint fa[] = {EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_RENDERABLE_TYPE,
                   EGL_OPENGL_ES2_BIT, EGL_SURFACE_TYPE, window ? EGL_WINDOW_BIT : EGL_PBUFFER_BIT,
                   EGL_NONE};
    EGLConfig all[64];
    GLint uc;
    double t0;
    int i;

    t0 = now_ms();
    if (!eglInitialize(d, &maj, &min)) {
        printf("eglInitialize failed: 0x%x\n", eglGetError());
        return 1;
    }
    printf("eglInitialize %d.%d in %.1f ms vendor=%s version=%s\n", maj, min, now_ms() - t0,
           eglQueryString(d, EGL_VENDOR), eglQueryString(d, EGL_VERSION));
    printf("EGL ext: %s\n", eglQueryString(d, EGL_EXTENSIONS));
    dump_configs(d);
    void *fbwin = NULL;
    int (*fmt_fn)(void) = NULL;
    void *(*create)(unsigned, unsigned, int) = NULL;
    int want = 0;
    if (window) {
        const char *lib = getenv("GLASS_NATIVE_WINDOW_LIB");
        fbwin = dlopen(lib ? lib : "libglass_fbwin.so", RTLD_NOW);
        if (!fbwin) {
            printf("dlopen fbwin: %s\n", dlerror());
            return 1;
        }
        fmt_fn = (int (*)(void))dlsym(fbwin, "glass_gfx_native_window_format");
        create = (void *(*)(unsigned, unsigned, int))dlsym(fbwin, "glass_gfx_create_native_window");
        if (!fmt_fn || !create) {
            printf("dlsym fbwin: %s\n", dlerror());
            return 1;
        }
        want = fmt_fn();
        printf("window format=%d\n", want);
        if (want < 0)
            return 1;
    }
    eglChooseConfig(d, fa, all, 64, &n);
    printf("choose: %d\n", n);
    if (n < 1)
        return 1;
    cfg = all[0];
    if (window) {
        for (i = 0; i < n; i++) {
            eglGetConfigAttrib(d, all[i], EGL_NATIVE_VISUAL_ID, &vis);
            if (vis == want) {
                cfg = all[i];
                break;
            }
        }
        printf("picked config index %d visual %d\n", i, vis);
    }
    eglBindAPI(EGL_OPENGL_ES_API);
    ctx = eglCreateContext(d, cfg, EGL_NO_CONTEXT, ca);
    if (ctx == EGL_NO_CONTEXT) {
        printf("eglCreateContext: 0x%x\n", eglGetError());
        return 1;
    }
    if (window) {
        void *win = create(W, H, want);
        if (!win) {
            printf("no native window\n");
            return 1;
        }
        s = eglCreateWindowSurface(d, cfg, (EGLNativeWindowType)win, NULL);
    } else {
        EGLint pa[] = {EGL_WIDTH, W, EGL_HEIGHT, H, EGL_NONE};
        s = eglCreatePbufferSurface(d, cfg, pa);
    }
    if (s == EGL_NO_SURFACE) {
        printf("create surface: 0x%x\n", eglGetError());
        return 1;
    }
    if (!eglMakeCurrent(d, s, s, ctx)) {
        printf("eglMakeCurrent: 0x%x\n", eglGetError());
        return 1;
    }
    printf("GL_RENDERER=%s\nGL_VENDOR=%s\nGL_VERSION=%s\nGLSL=%s\n", glGetString(GL_RENDERER),
           glGetString(GL_VENDOR), glGetString(GL_VERSION),
           glGetString(GL_SHADING_LANGUAGE_VERSION));
    printf("GL_EXT=%s\n", glGetString(GL_EXTENSIONS));
    uc = glGetUniformLocation(build(), "c");
    if (!window) {
        scene(uc, 0);
        glFinish();
        readback("pbuffer");
        return 0;
    }
    if (interval >= 0) {
        printf("swap interval %d: %d\n", interval, eglSwapInterval(d, interval));
    }
    if (frames < 1) {
        printf("no frames asked for\n");
        return 1;
    }
    double *tot = calloc(frames, sizeof(double)), *dcpu = calloc(frames, sizeof(double));
    if (!tot || !dcpu) {
        printf("out of memory\n");
        return 1;
    }
    double run0 = now_ms(), cpu0 = cpu_ms();
    for (i = 0; i < frames; i++) {
        double t = now_ms(), c = cpu_ms();
        scene(uc, i);
        if (i == frames - 1 || i == 0) {
            glFinish();
            readback(i ? "last-frame" : "first-frame");
        }
        if (!eglSwapBuffers(d, s)) {
            printf("swap %d: 0x%x\n", i, eglGetError());
            return 1;
        }
        tot[i] = now_ms() - t;
        dcpu[i] = cpu_ms() - c;
        if (i < 5 || i == 60)
            printf("frame %d: wall %.2f ms cpu %.2f ms\n", i, tot[i], dcpu[i]);
    }
    double wall = now_ms() - run0, cpu = cpu_ms() - cpu0;
    qsort(tot, frames, sizeof(double), cmp);
    qsort(dcpu, frames, sizeof(double), cmp);
    printf("RESULT frames=%d wall=%.0f ms fps=%.1f wall p50=%.2f p95=%.2f cpu p50=%.2f p95=%.2f "
           "cpu total=%.0f ms err=0x%x\n",
           frames, wall, frames * 1000.0 / wall, tot[frames / 2], tot[frames * 95 / 100],
           dcpu[frames / 2], dcpu[frames * 95 / 100], cpu, glGetError());
    usleep(100000);
    fb_report("after");
    return 0;
}

int main(int argc, char **argv)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_crash;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGBUS, &sa, NULL);
    setvbuf(stdout, NULL, _IOLBF, 0);
    if (argc >= 2 && !strcmp(argv[1], "pbuffer"))
        return run(0, 1, -1);
    if (argc >= 3 && !strcmp(argv[1], "window"))
        return run(1, atoi(argv[2]), argc > 3 ? atoi(argv[3]) : -1);
    if (argc >= 4 && !strcmp(argv[1], "scan"))
        return scan(strtoul(argv[2], NULL, 16), strtoul(argv[3], NULL, 10), argc > 4 ? argv[4] : NULL);
    printf("usage: gpu_probe scan BA_HEX PITCH [FILE]\n");
    printf("usage: gpu_probe pbuffer | window N [interval]\n");
    return 2;
}
