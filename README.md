# glass-apps

Glass apps for Google Glass Explorer Edition: the card timeline launcher at 640×360, settings, setup, camera and voice. Written in Rust on a thin software graphics layer, with no Android dependency, so the same apps run on an open Android build and on plain Linux. They talk to the Glass system services of [glass-services](https://github.com/OpenMyGlass/glass-services) through its [interface](https://github.com/OpenMyGlass/glass-services/blob/main/docs/api.md). Part of [OpenMyGlass](https://github.com/OpenMyGlass).

Plans and decisions are in [`docs/plans/`](docs/plans/).

## Crates

| Crate | Content |
|---|---|
| [`glass-gfx`](crates/glass-gfx) | `Frame` (RGBA8888, premultiplied), shapes and images with tiny-skia, text with fontdue (single line, wrapping, ellipsis), the `Display` trait; headless back end (memory, PNG files) and host window back end (`window` feature, minifb) |
| [`glass-ui`](crates/glass-ui) | Cards (home clock, text, settings entry, image) in Glass style; the timeline: gestures, live drag from `Scroll`, settle animation, cached per-card layers; `InputEvent` mirroring the glass-services messages |
| [`glass-launcher`](crates/glass-launcher) | The launcher binary: mock scripts headless, or a host window driven by the keyboard. Connecting to `glassd` through `glass-api` comes in a later change |

## Running on a desktop

Host window (needs a desktop; on Linux the X11/Wayland development packages, see CI):

```sh
cargo run -p glass-launcher --features window
```

Keys: **Right** = swipe forward (older cards), **Left** = swipe backward (settings side), **Enter**/**Space** = tap, **Down** = swipe down (back to home; on home, the screen turns off and the next key wakes it), **Esc** = quit. `--time 10:42` fixes the clock.

Headless, with a mock script in the `glassd --mock` syntax (`tap`, `swipe forward|backward|down|up`, `scroll 0.3`, `wait 200ms`, ...), writing every changed frame as a PNG:

```sh
cargo run -p glass-launcher -- --headless --script examples/demo.script --out /tmp/frames --time 10:42
```

## Tests

```sh
cargo test --workspace                                       # unit, timeline, card and golden-image tests
GLASS_UPDATE_GOLDEN=1 cargo test -p glass-ui --test golden   # regenerate tests/golden/*.png after a deliberate change
cargo test --release -p glass-ui --test bench -- --ignored --nocapture   # card render and scroll composite times
```

Golden images are compared with a tolerance of 3 per channel; on a mismatch the actual image is written to `target/golden-actual/`. Rendering is pure Rust with the SIMD paths of tiny-skia and fontdue disabled, so the images are the same on every platform; CI checks them on Linux x86-64 and on armv7 under QEMU (they were generated on macOS AArch64).

CI (`.github/workflows/ci.yml`): `host` (format, clippy with and without the window feature, tests, benchmarks, headless demo, dependency licences), `android-armv7` (`armv7-linux-androideabi`, API 22, the runner's NDK clang as linker), `linux-armv7` (static `armv7-unknown-linux-musleabihf` linked with `rust-lld`, tests and the demo under QEMU).

## Design notes

- **Timeline:** home in the middle, "now" cards and settings on the backward side, history on the forward side. A drag across the whole touchpad width moves 2 cards; a release settles on the nearest card after projecting the release velocity 0.15 s ahead, and a release faster than 0.5 pad widths/s moves at least one card. The swipe gesture that follows a drag (glass-services sends both) is not counted twice. Animations are ease-out cubic over 0.12 s + 0.12 s per card (at most 0.45 s), advanced only by `tick(dt)`, so they are deterministic. Details in [`timeline.rs`](crates/glass-ui/src/timeline.rs).
- **Rendering:** each card renders once into its own 640×360 layer and is re-rendered only when what it shows changes (content, clock minute, relative time text). At rest, the neighbours of the visible card are rendered ahead and other layers are dropped (at rest at most three, about 900 KiB each; a fling across several cards renders the cards it passes and holds their layers until the next rest); a scrolling frame is two row copies, with no card rendering.
- **Style:** after the public Glass design guidelines: black background, white Inter Light text, 40 px margins, body text sized 64/48/40/34 px to fit, footer in 26 px grey with the source on the left and the relative time on the right.

## Third-party code

The device builds (`armv7-linux-androideabi`, `armv7-unknown-linux-musleabihf`) contain only these crates besides our own, all under permissive licences:

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

The host window back end (`window` feature, not part of device builds) adds minifb (MIT OR Apache-2.0) and, on Linux, X11 and Wayland client crates (MIT; ISC for libloading; Unlicense OR MIT for memchr; MIT OR Apache-2.0 with the Unicode-3.0 terms for unicode-ident). `tools/check_licenses.sh` checks the full dependency tree of each build in CI against this permissive set.

Fonts: Inter 4.1 Light, SIL Open Font License 1.1, in [`assets/fonts/`](assets/fonts/) with the licence and the checksums of the release files.

## License

Apache-2.0 (see `LICENSE`). Bundled fonts keep their own licence (see `assets/fonts/`).
