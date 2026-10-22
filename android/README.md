# Android launcher wrapper (route 1, milestone 3)

The launcher runs on Android as a small Java app, `org.openmyglass.launcher`, whose only
activity is the HOME screen. Its `SurfaceView` hands the `Surface` to the native launcher,
`libglass_launcher_jni.so` ([`crates/glass-android`](../crates/glass-android)), which draws
it with EGL and GLES2 on the PowerVR SGX540 from its own render thread, and takes its input
from `glassd` (glass-services), not from Android.

One build serves both Android tracks: Track A (4.4.4 KitKat, ART) and Track B (6.0.1).
It targets **API 19** with **NDK r25c** (the last NDK that supports API 19) and compiles the
Java side against `android-19`'s `android.jar` (minSdk 19, targetSdk 22). No Google file is
in the repository or in the build output: the NDK, the SDK platform and build-tools are
build tools only, and no GDK is used.

## What it does

| Part | Where | What |
|---|---|---|
| `HomeActivity` | `launcher/src/` | `MAIN`/`HOME`/`DEFAULT`/`LAUNCHER`, `singleTask`, landscape, every config change handled, fullscreen; lifecycle and surface callbacks go to the native side |
| `NativeBridge` | `launcher/src/` | the JNI surface: `nativeCreate(filesDir)`, `nativeSurfaceCreated/Changed/Destroyed`, `nativeResume/Pause`, `nativeEvent(line)`, `nativeDestroy`; native requests arrive in `onNativeRequest(line)` on the render thread and are posted to the main thread |
| `AndroidPlatform` | `launcher/src/` | carries out platform requests (Wi-Fi and Bluetooth on/off, discoverable, pairing answers, `device_provisioned` and `user_setup_complete`, opening an app by package) and reports Wi-Fi, Bluetooth, pairing and set-up state as event lines |
| `PlatformLines` | `launcher/src/` | the tab-separated line format of both directions (same as `crates/glass-launcher/src/platform.rs`), tested on the JVM |
| render thread | `crates/glass-android` | the EGL window compositor (y-flipped into the window's default framebuffer, `eglSwapBuffers`), the `glassd` session (focus taken on resume, given back on pause, taken again if lost while resumed), the setup flow and dictation; blocks in `poll` on the command pipe and the `glassd` socket |

- **No software fallback:** if EGL or GLES fails, the error is logged (logcat tag
  `glass-launcher`, "GLES renderer failed: ...") and nothing is drawn.
- **Buffers:** the window's buffers are set to 640×360 in the EGL config's format
  (`ANativeWindow_setBuffersGeometry`), whatever the view's size.
- **Setup:** on first boot (no `setup-done` file in the app's files directory, and
  `device_provisioned` 0) the launcher shows the setup cards: welcome, pair the phone
  (Bluetooth on, discoverable for 300 s, the pairing code confirmed with a tap), open
  OpenMyGlass on the phone (until glassd's `CompanionState` says a phone is connected), done.
  A swipe down on the pairing and phone steps offers to skip.
- **Dictation:** the "Take a note" card (next to home on the backward side, only when glassd
  offers `dictation`) starts a free dictation through glassd's relay to the phone; the final
  transcript becomes a note card.

## For the device tree

`tools/android-dist.sh <dir>` (or the CI artifact `glass-apps-android-dist`) produces a
self-contained directory meant for `<aosp>/vendor/openmyglass/glass-apps/`:

| File | Installed as |
|---|---|
| `Android.mk` | modules `GlassLauncher` (APPS, `priv-app`, re-signed with the platform key, overrides `Launcher2 Launcher3 Home`, requires `libglass_launcher_jni`) and `libglass_launcher_jni` (prebuilt shared library, `/system/lib/libglass_launcher_jni.so`; KitKat does not extract system apps' JNI libraries) |
| `glass-apps.mk` | product makefile: adds both modules, stops the build with a message if the prebuilt files are missing |
| `GlassLauncher.apk`, `lib/libglass_launcher_jni.so` | the prebuilt files |
| `SOURCE`, `SHA256SUMS` | the glass-apps commit, NDK, API level and build-tools, and checksums |

The device tree needs (both branches, `android-4.4.4` and `android-6.0.1_r31`), in
`device.mk`:

```make
# OpenMyGlass launcher (glass-apps tools/android-dist.sh into vendor/openmyglass/glass-apps)
$(call inherit-product, vendor/openmyglass/glass-apps/glass-apps.mk)
```

`LOCAL_OVERRIDES_PACKAGES` already keeps `Launcher2`, `Launcher3` and `Home` out of the
image. Optionally, on the branch that has the lean list (`android-4.4.4`), in
`BoardConfig.mk`:

```make
GLASS_REMOVED_PACKAGES += Provision
```

(`Provision` is not needed: the launcher's HOME filter has a higher priority, 2, so
`Provision` would never run, and the launcher's setup flow sets `device_provisioned` and
`user_setup_complete` itself.) The launcher also needs glass-services' packaging (`glassd` with
`--allow-package org.openmyglass.launcher`, the socket, and on 6.0 the SELinux rules for
`platform_app` to connect to it): see glass-services' `android/`.

## Not checked without a Glass

Everything on a device: EGL window creation on the SGX540, the y-flip on the real panel,
surface loss and return (screen off, another app in front), the HOME role and
`singleTask` behaviour, pairing confirmation without the system dialog, `setScanMode`
(hidden API) for discoverability, `device_provisioned` writes, logcat output. CI checks the
build, the library's dependencies (API 19 libraries only, no link-time EGL/GLES), the JNI
entry points, the APK manifest and signature, and the render loop on the host with the
software compositor and glass-api's mock of glassd.
