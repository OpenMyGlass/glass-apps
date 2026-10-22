# GPU rendering (plan)

Owner decision of 2026-10-22 ([glass-rom roadmap](https://github.com/OpenMyGlass/glass-rom/blob/main/docs/plans/2026-10-21-open-source-roadmap.md), glass-rom PRs #19 and #20): **the UI renders on the GPU on every system.**
- Route 1 (Android): EGL and GLES2 on the XE24 PowerVR SGX540 libraries.
- Route 2 (plain Linux): the same Android PowerVR libraries, loaded through libhybris (setup on the device side, `pvrsrvinit` and gralloc access, is in the roadmap).
- Software rendering is a debugging aid only.

This replaces the M1 decision that made the software framebuffer the main path ([R5 plan](2026-10-21-r5-glass-apps.md), "UI" and "Thin graphics layer"). tiny-skia stays for golden tests, host development and a debug fallback.

## Design

The card and timeline code must not know which renderer draws it. Two seams:

1. **`Painter`** (draw commands): `clear`, `fill_rect`, `fill_rounded_rect`, `draw_image`, `draw_glyph_run`. Text layout (line breaking, glyph positions rounded to whole pixels) is split out of `glass-gfx`'s text module so both painters receive the same positioned glyphs. Card rendering in `glass-ui` is written against `Painter` only.
   - `SkiaPainter` draws into a `Frame` with tiny-skia (the M1 code).
   - `GlesPainter` draws into a GLES2 framebuffer object:
     - solid shapes with a small shader; rounded rects by a distance function in `highp` whenever `GL_FRAGMENT_PRECISION_HIGH` is defined (the SGX540 defines it); otherwise in `mediump` with coordinates relative to the nearest corner, so values stay within the corner radius;
     - images as textures, downscaled on the CPU to their drawn size before upload (no mipmaps on non-power-of-two textures, and photos exceed the SGX540's 2048 limit; `GL_MAX_TEXTURE_SIZE` is queried). `Frame` pixels are already premultiplied and are uploaded as they are;
     - text from a **glyph atlas**: an `ALPHA` texture (the shader reads `.a`), glyphs rasterized once by fontdue per (glyph, size), 1 px padding between glyphs, uploaded with `glTexSubImage2D` and `GL_UNPACK_ALIGNMENT` 1. At most 1024×1024; when it is full it is cleared and refilled for the current cards. The fragment shader outputs `colour_premultiplied × coverage`.
   - Images are not clipped to rounded corners (no stencil buffer is used, and the cards do not need it).
2. **`Compositor`** (layers and frames): `create_layer(w, h) -> LayerId`, `paint_layer(id, |painter| ...)`, `drop_layer`, `compose(&[(LayerId, x, y, opacity)])`, `present()`.
   - The software compositor keeps layers as `Frame`s and presents through the existing `Display` back ends (headless PNG, host window).
   - The GLES2 compositor keeps each card layer as a texture attached to an FBO. A frame is a full `glClear`, then one textured quad per visible layer (blending off for opaque layers, as the software path copies them), then `eglSwapBuffers`. A card is rendered when it changes or when the timeline first reaches it; moving an already rendered card is only a quad.
   - `Timeline::render` talks to a `Compositor`, so the layer cache and the "no card rendering on the scroll path at rest" rule hold for both renderers.

### GLES2 and SGX540 constraints

- GLES 2.0 only, no extensions required. Every shader starts with `#version 100`; fragment shaders default to `mediump`.
- Card layers are 640×360 textures (non-power-of-two): `CLAMP_TO_EDGE`, no mipmaps. **Sampling is `NEAREST`** for layers and the glyph atlas, which are drawn 1:1 at whole-pixel positions, so the GLES output matches the software output; `LINEAR` only for scaled images. Texture coordinates are `highp` varyings passed straight to `texture2D` (no arithmetic on them in the fragment shader, which also avoids dependent reads, slow on PowerVR).
- Layer FBOs use an RGBA/UNSIGNED_BYTE texture as colour attachment, with no depth or stencil. Core GLES2 allows an implementation to reject that, so `glCheckFramebufferStatus` is checked after each FBO is created; if it fails, opaque layers fall back to RGB565 textures, and if that fails too the renderer reports an error. With the RGB565 fallback the output no longer matches the goldens within ±2 (device only, on the checklist).
- Premultiplied alpha everywhere (`glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA)`), as in the software path.
- **Memory:** at most `MAX_RESIDENT_LAYERS` (4) card layers, enforced by evicting the layer farthest from the current card (a fling re-renders a card it reaches again). 4 × 900 KiB of layers, plus the 1 MiB atlas, plus 2 or 3 window buffers of 900 KiB: under 8 MiB of GPU memory.
- One context, one thread. No `glReadPixels` on the device path (tests only).
- **What CI cannot check:** Mesa llvmpipe supports non-power-of-two textures fully and computes `mediump` at 32-bit precision, so violations of the SGX540 limits would pass in CI. A debug assertion layer in `glass-gfx-gles` (on in tests) rejects: non-`CLAMP_TO_EDGE` wrap modes or mipmaps on NPOT textures, `glGenerateMipmap`, sizes above `GL_MAX_TEXTURE_SIZE`, shaders without `#version 100`. Precision is checked only on the device (checklist).

### Platform layer (EGL)

`glass-gfx-gles` uses `khronos-egl` with the `dynamic` feature (libloading) and `DynamicInstance<EGL1_4>` (Android's and libhybris' libEGL are EGL 1.4; the 1.5 and platform-display entry points are looked up optionally), and `glow` for GL. Core GLES2 functions are looked up with `dlsym` in `libGLESv2` first, and `eglGetProcAddress` only for extensions (before EGL 1.5 it need not return core functions). Library names: `libEGL.so`/`libGLESv2.so` on Android, `libEGL.so.1`/`libGLESv2.so.2` on Mesa and libhybris.

| Platform | Display and surface | Milestone |
|---|---|---|
| Host tests and CI | Mesa llvmpipe: `eglGetPlatformDisplay(EXT)` with `EGL_PLATFORM_SURFACELESS_MESA` when the client extensions offer it, else a pbuffer; context made current with `EGL_NO_SURFACE` (`EGL_KHR_surfaceless_context`); rendering into an FBO and reading it back | GPU-1 |
| Host development | the same, with the frame read back and copied to the `minifb` window (slow, fine for development) | GPU-1 |
| Android (route 1) | an `ANativeWindow` from the launcher wrapper's `Surface`: the renderer runs inside the wrapper's process, as a native library called through JNI (not the standalone binary). Done in M3: `GlesCompositor::for_window` (default display, a config with window and pbuffer bits so a 1×1 pbuffer keeps the context and layers while the surface is gone), `attach_window`/`detach_window`, frames composed into the default framebuffer with y flipped in the projection, `eglSwapBuffers`; the window's buffers are set to 640×360 in the config's native visual format | M3 |
| Plain Linux (route 2) | libhybris `libEGL` with the PowerVR libraries, an fbdev or hwcomposer window from libhybris | later, with glass-services' route-2 packaging (its M5) |

**Linking:** `dlopen` does not work in a statically linked musl binary, so the GPU path needs dynamically linked builds:
- Android: as now (bionic, dynamic).
- Plain Linux: CI adds an `armv7-unknown-linux-gnueabihf` build (dynamic, glibc). libhybris is proven mainly on glibc systems (Sailfish OS, Ubuntu Touch); route 2 is Alpine (musl), where libhybris is less tried. Whether route 2 uses Alpine with a dynamically linked musl build, or a glibc base, is checked on hardware (feasibility risk, recorded for the owner).
- The static musl build stays as the software-only debug build.

**No silent fallback:** `--renderer gles|software`. On device builds the default is `gles`, and a missing or failing EGL is a hard error with a clear message; `software` must be asked for. On host builds the default is `gles` when EGL initializes and `software` otherwise, with a log line.

**API level** (ruling of 2026-10-22, milestone 3): the roadmap made Track A (Android 4.4.4 KitKat with ART) the near-term target, so the launcher **does** target it. One Android build serves both tracks: `armv7-linux-androideabi` at **API 19** with **NDK r25c** (the last NDK that supports API 19; linker `armv7a-linux-androideabi19-clang`, linked with `--no-undefined` so every symbol resolves against API 19's libraries), loaded unchanged by 4.4 and 6.0. The Java wrapper compiles against `android-19` (minSdk 19, targetSdk 22). This replaces the earlier "API 22 only, KitKat later with r25". Rust's standard library refers to `dl_iterate_phdr`, which API 19 does not export (it arrived in API 21); `crates/glass-android/src/compat.rs` defines it (reporting no objects, used only to symbolize panic backtraces), so this is a shim, not a blocker. glass-services does the same for `glassd`.

### Tests

- The existing goldens stay the reference. The GLES2 path renders the same scenes on Mesa llvmpipe and is compared per scene:
  - solid fills, layers and text: at most ±2 per channel, no pixel above (NEAREST sampling and whole-pixel glyphs make this reachable);
  - anti-aliased shape edges (rounded rects): ±2 per channel for all but at most 1 % of the pixels, whose difference is bounded at 64 (distance-function coverage differs from tiny-skia's path coverage on edge pixels).
  - On a mismatch, the GLES image goes to `target/golden-actual-gles/` and is uploaded as a CI artifact.
- CI: a new job named **`gles`** on `ubuntu-24.04` (pinned, so the Mesa version changes only on purpose) installs `libegl1`, `libgles2`, `libegl-mesa0` and `libgl1-mesa-dri`, sets `LIBGL_ALWAYS_SOFTWARE=1` and `GALLIUM_DRIVER=llvmpipe`, logs `GL_RENDERER` and `GL_VERSION`, and runs the GL tests with `GLASS_GLES_TESTS=1`. Without that variable (macOS, other hosts) the GL tests are skipped, not failed. `gles` is added to the `protect-main` required checks, and a `linux-armv7-gnu` cross-build job is added and required too (it links with `gcc-arm-linux-gnueabihf` and its sysroot, or cargo-zigbuild, since `rust-lld` alone cannot link a glibc target).

## Order

1. **GPU-1:** text layout split; `Painter` and `Compositor` seams; the M1 software path moved behind them (goldens unchanged); `GlesPainter`, the GLES2 compositor and the assertion layer; the `gles` and `linux-armv7-gnu` CI jobs; launcher option `--renderer`.
2. **M1b:** the launcher connects to `glassd` through `glass-api`.
3. **M2:** settings and menus.

## Device checklist (not run: no Glass available)

- [ ] EGL initializes on the PowerVR libraries (route 1) and through libhybris (route 2), and `GL_RENDERER` names the SGX540.
- [ ] Every layer FBO is complete (`glCheckFramebufferStatus`), with RGBA; note if the RGB565 fallback is used.
- [ ] Rounded-rect edges and text show no precision banding (`mediump` on the SGX540).
- [ ] A scrolling frame takes under 4 ms on the GPU, measured as the CPU time of `glFinish` after the frame's draw calls in a debug build; the CPU is idle while the timeline is at rest.
- [ ] Text in the GLES path is as sharp as in the software path.
- [ ] M3, Android: `eglCreateWindowSurface` on the wrapper's `ANativeWindow` works on the SGX540 with the chosen config (window and pbuffer bits), and the 1×1 pbuffer keeps the context while the surface is gone.
- [ ] M3: the image is upright on the panel (the y-flip; CI checks it only by rendering with the window orientation off screen).
- [ ] M3: after the screen goes off and on, or another app comes to the front and leaves, the surface is re-attached and the timeline is redrawn without recreating layers; nothing is drawn while paused.
- [ ] M3: a failing EGL on the device logs "GLES renderer failed" and draws nothing (no software fallback).
