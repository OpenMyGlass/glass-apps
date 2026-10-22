# glass-apps

Glass apps for Google Glass Explorer Edition: the card timeline launcher at 640×360, settings, setup, camera and voice. Written in Rust, rendering on the GPU with GLES2 (the software renderer is a debugging aid and the test reference), with no Android dependency, so the same apps run on an open Android build and on plain Linux. They talk to the Glass system services of [glass-services](https://github.com/OpenMyGlass/glass-services) through its [interface](https://github.com/OpenMyGlass/glass-services/blob/main/docs/api.md). Part of [OpenMyGlass](https://github.com/OpenMyGlass).

Plans and decisions are in [`docs/plans/`](docs/plans/).

## Crates

| Crate | Content |
|---|---|
| [`glass-gfx`](crates/glass-gfx) | The `Painter` (draw commands) and `Compositor` (layers, frames) traits; text layout into whole-pixel glyph runs shared by both renderers (single line, wrapping, ellipsis); the software renderer: `Frame` (RGBA8888, premultiplied), `SkiaPainter` (tiny-skia, fontdue), `SoftwareCompositor`; the `Display` trait with a headless back end (memory, PNG files) and a host window back end (`window` feature, minifb) |
| [`glass-gfx-gles`](crates/glass-gfx-gles) | The GLES2 renderer: EGL and GLES loaded at run time (khronos-egl, glow), off-screen contexts (Mesa surfaceless or pbuffer), `GlesPainter` (distance-function rounded rects, CPU-downscaled image textures, ALPHA glyph atlas), `GlesCompositor` (FBO texture layers), the SGX540 assertion layer |
| [`glass-ui`](crates/glass-ui) | Cards (home clock, text, settings entry, image) in Glass style, drawn through `Painter`; the timeline: gestures, live drag from `Scroll`, settle animation, cached per-card layers through `Compositor` (at most 4); `InputEvent` mirroring the glass-services messages |
| [`glass-launcher`](crates/glass-launcher) | The launcher binary: a `glassd` client (`--glassd`, through glass-services' `glass-api`: focus, gestures, screen state, reconnection), mock scripts headless, or a host window driven by the keyboard; `--renderer gles\|software` |

## Running on a desktop

Host window (needs a desktop; on Linux the X11/Wayland development packages, see CI):

```sh
cargo run -p glass-launcher --features window
```

The renderer is chosen with `--renderer gles|software`. On a host, the default is `gles` when EGL initializes (Mesa: `libEGL.so.1`, `libGLESv2.so.2`; GLES frames are read back into the window or the PNG files) and `software` otherwise, with a log line. Device builds (Android, Linux on ARM) default to `gles` and stop with an error when EGL fails; `software` must be asked for. The static musl build is software only (built with `--no-default-features`, without the `gles` feature).

Keys: **Right** = swipe forward (older cards), **Left** = swipe backward (settings side), **Enter**/**Space** = tap, **Down** = swipe down (back to home; on home, the simulated screen turns off; Enter, Space, Left or Right wakes it and is consumed, as `glassd` does), **Up** = swipe up (no effect), **Esc** = quit. `--time 10:42` fixes the clock.

Headless, with a mock script in the `glassd --mock` syntax (`tap`, `swipe forward|backward|down|up`, `scroll 0.3`, `wait 200ms`, ...), writing every changed frame as a PNG:

```sh
cargo run -p glass-launcher -- --headless --script examples/demo.script --out /tmp/frames --time 10:42
```

### Against `glassd --mock`

The launcher takes its input and the screen state from `glassd` of [glass-services](https://github.com/OpenMyGlass/glass-services) with `--glassd [SOCKET]` (default: `$GLASS_SERVICES_SOCKET`, else `/run/glassd/glassd.sock` on Linux and `/dev/socket/glassd` on Android). On a desktop, in two terminals (keep the socket path short: Unix socket paths are limited to about 100 bytes):

```sh
# terminal 1, in a glass-services checkout: type commands such as tap, swipe forward, swipe down, scroll 0.3
cargo run -p glassd -- --mock --socket /tmp/glassd.sock -v
# terminal 2, here: a host window (or --out DIR for PNG files, or neither)
cargo run -p glass-launcher --features window -- --glassd /tmp/glassd.sock
```

In this mode the keyboard does nothing; `glassd` owns the screen: a one-finger swipe down on the home card sends `ScreenRequest{ACTION_SLEEP}`, nothing is drawn while the screen is off, and on a wake the launcher shows the home card unless the last sleep was a timeout or a gesture (nudge, two-finger swipe down) at most 5 s ago (a swipe wake always shows home). Input is ignored while another client holds the input focus. If `glassd` goes away the launcher reconnects, with a delay from 100 ms doubling up to 5 s. The loop blocks on the socket: one frame at a time while the timeline moves, until the next minute (the home clock) at rest, and with no timeout while the screen is off. `tools/e2e_glassd.sh GLASSD LAUNCHER` runs a scripted session (CI runs it in the `host` job against glassd built at the pinned revision).

## Tests

```sh
cargo test --workspace                                       # unit, timeline, card and golden-image tests
GLASS_UPDATE_GOLDEN=1 cargo test -p glass-ui --test golden   # regenerate tests/golden/*.png after a deliberate change
GLASS_GLES_TESTS=1 cargo test -p glass-gfx-gles -- --nocapture --test-threads=1   # GLES2 renderer against the goldens (needs EGL)
GLASS_GLES_TESTS=1 cargo test -p glass-launcher --test golden_settings -- --nocapture   # settings cards and menus on GLES2
cargo test --release -p glass-ui --test bench -- --ignored --nocapture   # card render and scroll composite times
```

Golden images are compared with a tolerance of 3 per channel; on a mismatch the actual image is written to `target/golden-actual/`. Rendering is pure Rust with the SIMD paths of tiny-skia and fontdue disabled, so the images are the same on every platform; CI checks them on Linux x86-64 and on armv7 under QEMU (they were generated on macOS AArch64).

The GLES tests render the same scenes and compare them with the same goldens: solid fills, layers and text within ±2 per channel on every pixel; anti-aliased rounded-rect edges within ±2 except on at most 1 % of the pixels, which stay within 64. Mismatching GLES images go to `target/golden-actual-gles/`. Without `GLASS_GLES_TESTS=1` (macOS, hosts without EGL) they are skipped. The SGX540 assertion layer is on in tests: NPOT textures without `CLAMP_TO_EDGE` or with mipmaps, `glGenerateMipmap`, textures above `GL_MAX_TEXTURE_SIZE` and shaders without `#version 100` panic. Precision (`mediump` on the SGX540) can only be checked on the device.

CI (`.github/workflows/ci.yml`): `host` (format, clippy with and without the window feature and without `gles`, tests, benchmarks, headless demo, dependency licences), `gles` (ubuntu-24.04, Mesa llvmpipe: GL_RENDERER/GL_VERSION, the GLES tests, a headless demo with `--renderer gles`), `android-armv7` (`armv7-linux-androideabi`, API 22, the runner's NDK clang as linker), `linux-armv7-gnu` (dynamic `armv7-unknown-linux-gnueabihf` linked with `gcc-arm-linux-gnueabihf`, the GLES-capable plain-Linux build; checks the hard error without EGL and runs the software demo under QEMU), `linux-armv7` (static `armv7-unknown-linux-musleabihf` linked with `rust-lld`, the software-only debug build, tests and the demo under QEMU).

## Design notes

- **Timeline:** home in the middle, "now" cards and settings on the backward side, history on the forward side. A drag across the whole touchpad width moves 2 cards; a release settles on the nearest card after projecting the release velocity 0.15 s ahead, and a release faster than 0.5 pad widths/s moves at least one card. The swipe gesture that follows a drag (glass-services sends both) is not counted twice. Animations are ease-out cubic over 0.12 s + 0.12 s per card (at most 0.45 s), advanced only by `tick(dt)`, so they are deterministic. Details in [`timeline.rs`](crates/glass-ui/src/timeline.rs).
- **Rendering:** the GPU plan is [`docs/plans/2026-10-22-gpu-rendering.md`](docs/plans/2026-10-22-gpu-rendering.md). Card and timeline code draw only through `Painter` and `Compositor`, so both renderers draw the same cards. Each card renders once into its own 640×360 layer and is re-rendered only when what it shows changes (content, clock minute, relative time text). At rest, the neighbours of the visible card are rendered ahead and other layers are dropped (three at rest, about 900 KiB each); a fling renders the cards it reaches, and never more than four layers exist (the one farthest from the position is dropped first). A scrolling frame is two layer copies (row copies in software, two quads with blending off on the GPU), with no card rendering.
- **Menus:** `TAP` on a card opens its menu over the timeline: one item per screen in large text (no icons), with position dots; swipes move between items (a drag follows the finger), `TAP` chooses, `SWIPE_DOWN` closes. Text and image cards have "Delete"; the home and device cards have no menu. An open menu frees the timeline's layers, so at most four layers exist in all; the screen turning off closes it.
- **Settings:** cards on the backward side after the "now" cards: Wi-Fi, Bluetooth, screen timeout, head wake, wink, on-head detection, nudge to sleep, device. Their menus turn a switch on or off, pick a screen timeout (5 s to 10 min), or open the wake-angle screen (10° to 40° in 5° steps by swipes, `TAP` keeps, `SWIPE_DOWN` cancels). A choice sends `SetSettings` with only that field; the cards show what `glassd` applied (its answer and broadcasts, so a clamped value shows as clamped, and a change by another client shows too), "Waiting for glassd" before its snapshot. Without `glassd` (scripts, the keyboard window) the change is applied locally with the same ranges. Wi-Fi and Bluetooth go through the `Platform` trait ([`platform.rs`](crates/glass-launcher/src/platform.rs)); only an in-memory mock exists until the Android glue (M3) and connman or iwd and BlueZ (route 2). The device card lists the launcher version and renderer, `glassd`'s version and capabilities, and the platform.
- **Style:** after the public Glass design guidelines: black background, white Inter Light text, 40 px margins, body text sized 64/48/40/34 px to fit, footer in 26 px grey with the source on the left and the relative time on the right.

## Third-party code

The device builds (`armv7-linux-androideabi`, `armv7-unknown-linux-gnueabihf`, `armv7-unknown-linux-musleabihf`) contain only these crates besides our own, all under permissive licences:

| Crate | Licence |
|---|---|
| tiny-skia, tiny-skia-path | BSD-3-Clause |
| fontdue | MIT OR Apache-2.0 OR Zlib |
| ttf-parser | MIT OR Apache-2.0 |
| png, fdeflate, flate2, crc32fast | MIT OR Apache-2.0 |
| miniz_oxide | MIT OR Zlib OR Apache-2.0 |
| adler2 | 0BSD OR MIT OR Apache-2.0 |
| simd-adler32, strict-num, core_maths, libm | MIT |
| arrayref | BSD-2-Clause |
| arrayvec, bitflags, cfg-if, log, hashbrown, allocator-api2, equivalent, libc | MIT OR Apache-2.0 |
| bytemuck | Zlib OR Apache-2.0 OR MIT |
| foldhash | Zlib |
| khronos-egl (GLES builds) | MIT OR Apache-2.0 |
| glow (GLES builds) | MIT OR Apache-2.0 OR Zlib |
| libloading (GLES builds) | ISC |
| glass-api (glass-services, pinned git revision) | Apache-2.0 |
| prost | Apache-2.0 |
| bytes | MIT |
| anyhow, either, itertools | MIT OR Apache-2.0 |

The host window back end (`window` feature, not part of device builds) adds minifb (MIT OR Apache-2.0) and, on Linux, X11 and Wayland client crates (MIT; ISC for libloading; Unlicense OR MIT for memchr; MIT OR Apache-2.0 with the Unicode-3.0 terms for unicode-ident). glass-api generates its message types at build time with prost-build and protox (no `protoc`); these run on the build host only (Apache-2.0, MIT OR Apache-2.0, Unlicense OR MIT for memchr and aho-corasick, and prost-derive's proc-macro crates). `tools/check_licenses.sh` checks the full dependency tree of each build in CI against this permissive set.

Fonts: Inter 4.1 Light, SIL Open Font License 1.1, in [`assets/fonts/`](assets/fonts/) with the licence and the checksums of the release files.

## License

Apache-2.0 (see `LICENSE`). Bundled fonts keep their own licence (see `assets/fonts/`).
