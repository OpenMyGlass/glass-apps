# GPU rendering (plan)

Owner decision of 2026-10-22 ([glass-rom roadmap](https://github.com/OpenMyGlass/glass-rom/blob/main/docs/plans/2026-10-21-open-source-roadmap.md), glass-rom PRs #19 and #20): **the UI renders on the GPU on every system.**
- Route 1 (Android): EGL and GLES2 on the XE24 PowerVR SGX540 libraries.
- Route 2 (plain Linux): the same Android PowerVR libraries, loaded through libhybris.
- Software rendering is a debugging aid only.

This replaces the M1 decision that made the software framebuffer the main path ([R5 plan](2026-10-21-r5-glass-apps.md), "UI" and "Thin graphics layer"). tiny-skia stays for golden tests, host development and a debug fallback.

## Design

The card and timeline code must not know which renderer draws it. Two seams:

1. **`Painter`** (draw commands): `clear`, `fill_rect`, `fill_rounded_rect`, `draw_image`, `draw_text` (with the layout already done by `glass-gfx`'s text module). Card rendering in `glass-ui` is written against `Painter` only.
   - `SkiaPainter` draws into a `Frame` with tiny-skia (the M1 code).
   - `GlesPainter` draws into a GLES2 framebuffer object: solid shapes with a small shader (rounded rects by a distance function in the fragment shader), images as textures, text from a **glyph atlas** (glyphs rasterized once by fontdue at the sizes used, uploaded to one `ALPHA`/`LUMINANCE` texture, drawn as textured quads with premultiplied colour).
2. **`Compositor`** (layers and frames): `create_layer(w, h) -> LayerId`, `paint_layer(id, |painter| ...)`, `drop_layer`, `compose(&[(LayerId, x, y, opacity)])`, `present()`.
   - The software compositor keeps layers as `Frame`s and presents through the existing `Display` back ends (headless PNG, host window).
   - The GLES2 compositor keeps each card layer as an FBO-backed texture; a frame is a clear plus one textured quad per visible layer, then `eglSwapBuffers`. Scrolling never re-renders a card.
   - `Timeline::render` talks to a `Compositor`, so the layer cache and the "no card rendering on the scroll path" rule hold for both.

### GLES2 and SGX540 constraints

- GLES 2.0 only, no extensions required. Shaders in `#version 100`, `mediump` in fragment shaders.
- Textures: card layers are 640×360 (non-power-of-two): `CLAMP_TO_EDGE`, no mipmaps, `LINEAR` filtering, which GLES2 allows for NPOT textures. Glyph atlas 1024×1024 at most. Total texture memory for three card layers plus the atlas stays under 4 MB.
- Premultiplied alpha everywhere (`glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA)`), the same as the software path, so both produce the same pixels within rounding.
- One context, one thread. No `glReadPixels` on the device path (tests only).

### Platform layer (EGL)

`glass-gfx-gles` uses `khronos-egl` (dynamic loading of `libEGL`) and `glow` (GL function loading). EGL setup is separate from drawing:

| Platform | Display and surface | Milestone |
|---|---|---|
| Host tests and CI | Mesa (llvmpipe), EGL `EGL_PLATFORM_SURFACELESS_MESA` or a pbuffer; rendering into an FBO and reading it back | GPU-1 |
| Host development | the same, with the frame copied to the `minifb` window (slow, fine for development) | GPU-1 |
| Android (route 1) | `ANativeWindow` from the launcher wrapper's `Surface`, through the Android glue | M3 |
| Plain Linux (route 2) | libhybris `libEGL` with the PowerVR libraries, `fbdev` or hwcomposer window from libhybris | later, with glass-services' route-2 packaging (its M5) |

### Tests

- The existing goldens stay the reference. The GLES2 path renders the same scenes on Mesa llvmpipe and is compared with the goldens within a tolerance (per-channel difference and a maximum share of differing pixels; anti-aliased edges and glyph rasterization differ slightly between the two paths).
- CI: a `gles` job on Ubuntu installs Mesa (`libegl1`, `libgles2`, `mesa-utils`/`libegl-mesa0`) and runs the GL tests with `GLASS_GLES_TESTS=1`. Without that variable (macOS, other hosts) the GL tests are skipped, not failed. The job is added to the required checks.
- Cross-builds: `glass-gfx-gles` loads `libEGL`/`libGLESv2` at run time, so the armv7 Android and musl builds need no GL libraries at link time.

## Order

1. **GPU-1:** `Painter` and `Compositor` seams; the M1 software path moved behind them (goldens unchanged); `GlesPainter` and the GLES2 compositor; Mesa CI job; launcher option `--renderer gles|software` (default `gles` where EGL is available, `software` otherwise, with a log line).
2. **M1b:** the launcher connects to `glassd` through `glass-api`.
3. **M2:** settings and menus.

## Device checklist (not run: no Glass available)

- [ ] EGL initializes on the PowerVR libraries (route 1) and through libhybris (route 2), and `GL_RENDERER` names the SGX540.
- [ ] A scrolling frame takes under 4 ms of GPU time; the CPU is idle while the timeline is at rest.
- [ ] Text in the GLES path is as sharp as in the software path.
