# Framebuffer window for Model B's GPU launcher

`libglass_fbwin.so` gives a bionic process an `ANativeWindow` over the stock gralloc framebuffer HAL, with no SurfaceFlinger. `gpu_probe` is the smallest GLES2 client on the same path. `build.sh OUT_DIR` builds both for the stock Android linker (`ANDROID_NDK_HOME` = NDK r25c, API 19, sysv hash). The sources contain no Google or Imagination code; the blobs they load stay outside git.

Measured on the Glass (2026-11-05):

- `framebuffer_device_t` has a `flags` field before `width`, and reports `numFramebuffers` 0. The third `alloc` fails with `-ENOMEM`, so the window has 2 buffers. Stride is 1024 pixels: OMAPLFB keeps its own TILER buffers.
- XE24's `libEGL` calls `setSwapInterval` at window offset 68 and `perform` at 88, one word later than the AOSP 4.4 header gives. `glass_xe24_pad` matches that.
- While the window is in use, DISPC scans TILER (`DISPC_OVL_BA0(GFX)` is `0x10000000` or `0x10001000`, pitch 32768), not `/dev/fb0`. The CPU reads the same buffer at BA0 + `0x60000000` through `/dev/mem`. After the process exits BA0 is `0x9f800000` again.
