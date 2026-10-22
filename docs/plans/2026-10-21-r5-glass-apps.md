# R5: Glass apps (plan)

Roadmap phase R5 of OpenMyGlass ([glass-rom roadmap](https://github.com/OpenMyGlass/glass-rom/blob/main/docs/plans/2026-10-21-open-source-roadmap.md)): the card timeline launcher at 640×360, settings, setup, camera, and voice with an open offline hotword; dictation goes through the OpenMyGlass companion to the phone.

## Constraints

- **No Android dependency in the apps.** The UI and app logic run on route 1 (open Android) and route 2 (plain Linux). They talk to Glass services only through the [glass-services interface](https://github.com/OpenMyGlass/glass-services/blob/main/docs/api.md), and draw only through a thin graphics layer.
- **Android glue in its own module** (later milestone): the launcher wrapper (a `HOME` activity that hands its `Surface` to the native UI), intents to open Android apps, and the camera and audio paths that need Android services.
- **Hardware:** two Cortex-A9 at up to 1 GHz, SGX540 (GLES2). The UI renders on the GPU on both routes: on Android through the PowerVR libraries, on plain Linux through libhybris loading the same libraries (owner decision 2026-10-22, see [`2026-10-22-gpu-rendering.md`](2026-10-22-gpu-rendering.md)). The display is 640×360.
- **Clean room:** Glass behaviour comes from public documentation (GDK reference, Glass design guidelines: cards, timeline, menus, typography); no stock APK is decompiled for this phase.

## Decisions

### Language: Rust

The same as glass-services ([its plan](https://github.com/OpenMyGlass/glass-services/blob/main/docs/plans/2026-10-21-r4-glass-services.md)), so the apps use the `glass-api` crate directly, and one toolchain cross-builds both repos.

### UI: our own small card renderer

> **Changed on 2026-10-22:** the owner decided that the UI renders on the GPU on every system. The GLES2 back end is now the primary renderer and the software path below is for tests, host development and debugging. See [`2026-10-22-gpu-rendering.md`](2026-10-22-gpu-rendering.md).

| Option | For | Against |
|---|---|---|
| **Own renderer** on `tiny-skia` (BSD-3-Clause) and `fontdue` (MIT, Apache-2.0 or Zlib) (chosen) | Pure Rust, no C dependencies, cross-builds trivially; the Glass UI is simple (full-screen cards, large text, images, a horizontal timeline); a retained layer per card makes scrolling a blit; deterministic output, so tests compare pixels | We write layout and animation ourselves |
| Slint | Declarative, embedded-friendly, software renderer | On embedded devices the choice is GPLv3 or a paid licence (the royalty-free licence excludes embedded systems): awkward for an Apache-2.0 project others may redistribute |
| LVGL | MIT, made for software rendering on small CPUs | C: the UI would be written in C or through immature bindings |
| Qt, GTK, Flutter | Complete toolkits | Large; heavy dependency trees for a 640×360 card UI |
| egui, iced | Rust | Immediate mode (egui) redraws everything each frame; iced pulls in winit and a large dependency tree |

The cost of software rendering at 640×360: one frame is 230,400 pixels (900 KiB in RGBA8888). A card is rendered once into its own layer; while the timeline scrolls, a frame is two layer blits, a few milliseconds on a Cortex-A9 (blits are plain memory copies). Text and images are drawn only when a card changes. tiny-skia has SIMD only for x86 and AArch64, so on armv7 card rendering runs its scalar path: it must stay off the scroll path (cards are rendered ahead, when they change or come next to the visible one), and only the visible card and its neighbours keep a layer (about 900 KiB each), so memory stays bounded.

### Thin graphics layer

The `glass-gfx` crate defines a `Frame` (an RGBA8888 pixel buffer of the display's size) and a `Display` trait with `present(&Frame)`. Back ends:

| Back end | Where | Milestone |
|---|---|---|
| Headless (PNG files, in-memory) | tests, CI | M1 |
| Host window (`minifb`, behind a Cargo feature) | development on a desktop, with the keyboard driving gestures through the mock | M1 |
| Linux framebuffer, debug fallback only (`/dev/fb0`, `mmap`, the pixel format reported by `FBIOGET_VSCREENINFO`; double buffering through `yres_virtual` and `FBIOPAN_DISPLAY`, `FBIO_WAITFORVSYNC` when the driver has it; one conversion pass from premultiplied RGBA to the panel's format per frame, measured) | route 2 | M2 |
| DRM dumb buffers (`omapdrm`) | route 2 on a mainline kernel (glass-mainline) | later |
| Android `ANativeWindow` (EGL window surface for the GLES2 renderer; software lock and post only as a debug fallback), in the Android glue | route 1 | M3 |
| GLES2 (card layers as textures, scrolling as textured quads) | **primary renderer** on routes 1 and 2 (libhybris on route 2); host tests on Mesa | GPU-1, see [`2026-10-22-gpu-rendering.md`](2026-10-22-gpu-rendering.md) |

### Fonts

Glass used Roboto Light and Thin. We bundle **Inter** (Rasmus Andersson, SIL Open Font License 1.1) instead, a similar open sans-serif with Light and Thin weights, in `assets/fonts/` with its licence. Roboto itself is Apache-2.0 and would also be allowed, but Inter keeps the repository free of any file published by Google, which keeps rule 1 simple to check. The owner can switch to Roboto later; only the font files change.

### Voice

- **Hotword ("ok glass"-like):** an open, offline engine with a licence-clean model. The choice and licence check are in [`2026-10-21-voice.md`](2026-10-21-voice.md).
- **Commands:** a small fixed grammar on Glass ("take a picture", "record a video", ...).
- **Dictation:** through glass-services' dictation request to the OpenMyGlass companion, which uses its existing bridge messages (`DictationStart`, `AudioChunk`, `DictationEnd`, `Transcript`). If the bridge protocol needs a change (free dictation that is not a notification reply), it is proposed as a small PR in OpenMyGlass, not forked here.

## Crates

| Crate | Content | Milestone |
|---|---|---|
| `glass-gfx` | `Frame`, drawing primitives, text, `Display` trait, headless and host-window back ends | M1 |
| `glass-ui` | Cards (layout and rendering), the timeline (model, scrolling physics, rendering) | M1 |
| `glass-launcher` | The launcher binary: connects to `glassd`, holds input focus, shows the timeline | M1 |
| `glass-settings`, `glass-setup`, `glass-camera`, `glass-voice` | The other apps | M2 and later |

## Milestone 1 (this plan)

1. The timeline model: a home card (clock) in the middle, newer and settings cards on one side, older cards on the other, navigated with `SWIPE_FORWARD` and `SWIPE_BACKWARD` and followed live with `Scroll`; `TAP` opens a card's menu (placeholder in M1); `SWIPE_DOWN` on home sends `ScreenRequest{SLEEP}`.
2. Card rendering at 640×360 in Glass style (black background, white Inter Light text, footer with timestamp), with golden-image tests (rendered PNG compared with a committed reference, small tolerance).
3. `glass-launcher --headless --script FILE` runs a mock input script in the `glassd --mock` command syntax and writes the frames as PNG files; with `--features window` it opens a host window and maps the arrow keys and Enter to gestures. The launcher maps its input onto a local `InputEvent` that mirrors the glass-services messages 1:1. **M1b:** connect to `glassd` through the `glass-api` crate (and its in-process mock in tests) once that crate is merged in glass-services: take the input focus, convert `Gesture`/`Scroll`, send `ScreenRequest{ACTION_SLEEP}`, follow `ScreenState`.
4. Tests: timeline navigation from gesture sequences, scroll settling, rendering of each card type. Benchmarks (reported, not enforced): composite time of a scrolling frame and render time of one card, on the host now and on armv7 when a device is available.
5. CI: host tests, cross-builds for `armv7-linux-androideabi` and `armv7-unknown-linux-musleabihf`.

## Later milestones

- **GPU-1** (before M1b): the GLES2 renderer becomes the primary path, see [`2026-10-22-gpu-rendering.md`](2026-10-22-gpu-rendering.md).
- **M1b:** the launcher connects to `glassd` (see item 3 above).
- **M2:** settings (head wake, wink, screen timeout through `SetSettings`; Wi-Fi and Bluetooth through a small platform trait), menus; the Linux framebuffer back end only as a debug fallback. Menus, settings cards, the wake-angle screen, the device card and the `Platform` seam (mock only) are done; the framebuffer back end is not (it is a debug fallback, and the GPU path comes first).
- **M3:** Android glue (launcher wrapper, `ANativeWindow`, intents), setup (pairing with the OpenMyGlass phone app), dictation.
- **M4:** camera (through the platform: Android camera API or V4L2/Ducati on Linux), voice hotword and commands.

## Device checklist (not run: no Glass available)

- [ ] The launcher draws at 640×360 on the panel, text is sharp, the timeline scrolls without visible stutter.
- [ ] Frame time while scrolling, render time of one card, CPU use while idle (only the home clock redraws, once a minute).
- [ ] M2 menus: `TAP` on a text card opens its menu, swipes move between items as smoothly as the timeline, `SWIPE_DOWN` closes; text in menus is sharp on GLES and in software.
- [ ] M2 settings, each against a real `glassd`: head wake off and on (look-up no longer wakes, then wakes again); wake angle changed to 20° and 40° (look-up needs a lower or higher pitch); wink on (needs the glasshub calibration); on-head detection on (taking Glass off turns the screen off); nudge to sleep on; every screen timeout choice (5 s and 10 min timed). Each card shows the value `glassd` applied.
- [ ] M2 device card: the launcher version, renderer `gles`, and `glassd`'s capabilities on the device (touchpad, screen, wear, and head/wink when found).
- [ ] M2 Wi-Fi and Bluetooth cards: only the mock exists; with a real platform back end (M3 or route 2), turning each radio off and on and the shown network and devices.
