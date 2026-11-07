# Framebuffer window for Model B's GPU launcher

`libglass_fbwin.so` gives a bionic process an `ANativeWindow` over the stock gralloc framebuffer HAL, with no SurfaceFlinger. `gpu_probe` is the smallest GLES2 client on the same path. `build.sh OUT_DIR` builds both for the stock Android linker (`ANDROID_NDK_HOME` = NDK r25c, API 19, sysv hash). The sources contain no Google or Imagination code; the blobs they load stay outside git.

Measured on the Glass (2026-11-05):

- `framebuffer_device_t` has a `flags` field before `width`, and reports `numFramebuffers` 0. The third `alloc` fails with `-ENOMEM`, so the window has 2 buffers. Stride is 1024 pixels: OMAPLFB keeps its own TILER buffers.
- XE24's `libEGL` calls `setSwapInterval` at window offset 68 and `perform` at 88, one word later than the AOSP 4.4 header gives. `glass_xe24_pad` matches that.
- While the window is in use, DISPC scans TILER (`DISPC_OVL_BA0(GFX)` is `0x10000000` or `0x10001000`, pitch 32768), not `/dev/fb0`. The CPU reads the same buffer at BA0 + `0x60000000` through `/dev/mem`. After the process exits BA0 is `0x9f800000` again.

## Running it on Model B

`glass-launcher-supervisor.sh` starts the launcher: on the GPU when that can work, on the software renderer otherwise. Its header gives the rules; `test_supervisor.sh` checks them on a host with fake launchers (a process that cannot be killed is simulated with a `kill` that does nothing).

What the init must do:

| Requirement | Why |
|---|---|
| Mount `/var/lib/glass-launcher` (`GLASS_LAUNCHER_STATE`) on storage that survives a reboot, before starting the supervisor | The failure counters must outlive a kernel crash, or a GPU that panics is a boot loop. On `rootfs`, `ramfs`, `tmpfs` or a filesystem not known to persist (read from `/proc/mounts`) the supervisor refuses the GPU and starts the software launcher. |
| Send `SIGTERM` to the supervisor on a clean reboot or shutdown, before unmounting that storage | A stop in the first 20 s is otherwise counted as a GPU failure. |
| Test images booted from RAM only: `export GLASS_GPU_ALLOW_VOLATILE_STATE=1` before starting the supervisor | Without a mount they would run in software. The counters then reset at every boot, so a GPU kernel crash is a boot loop: never set this in a production image. |

Two GPU failures in a row, or two reboots for a stuck driver, write the `no-gpu` marker. A power cut in the first 20 s counts too, since it cannot be told from a crash. `glass-launcher-supervisor.sh --clear-gpu-state` removes the marker and the counters without a reinstall, and the next start tries the GPU. The UI shows no sign of the marker yet; the log does.

Limits of the bookkeeping: a kernel crash after the first 20 s is not counted, so one that recurs later than that on every boot still reboots each time. A bring-up process stuck in D state stays until the next boot and may keep the Glass out of deep sleep, because the freezer waits for it.

The fallback is invisible: a Glass that runs in software after a refused or failed GPU looks the same as one on the GPU, only slower, and only the log says why. The launchers' output goes through a relay in the supervisor, so a dead log reader cannot hand them `EPIPE`; once the reader is gone the relay writes to `/dev/kmsg`, and a relay that dies is started again within a second. After a launcher stops beating it gets 15 s to die before the driver is taken for stuck, and the reboot is called off if it dies before then.

The stock files come from the owner's own Glass and stay local, never in git or in a published image:

```
tools/extract-gpu-blobs.sh adb:SERIAL OUT_DIR    # or the factory system.img, or a copy of /system
```

It verifies each file against `gpu-blobs.sha256` (hashes only) and refuses an `OUT_DIR` inside a git work tree. Copy `OUT_DIR` over the image root, and add these, ours:

| File | Path on the Glass |
|---|---|
| The launcher built for bionic with `gles,fb,sound` | `/usr/libexec/glass-launcher-gles` |
| The static musl launcher | `/usr/bin/glass-launcher` |
| `libglass_fbwin.so` | `/usr/lib/glass/libglass_fbwin.so` |
| `pvrsrvkm_sgx540_120.ko`, from the same build as the kernel | `/lib/modules/` |
| `glass-launcher-supervisor.sh` | anywhere; the init runs it instead of the launcher |

Measured on the Glass (2026-11-05, real `glassd`, cable in, RAM boot): no blobs, or the `no-gpu` marker, go to software at once. Three kills in a row in the first seconds write the marker. A launcher stopped with `SIGSTOP` is replaced by the software one in 24 s. After a GPU run the software launcher scans out from `/dev/fb0`. Idle GPU launcher: 0 CPU ticks in 30 s with 6 heartbeats. Not run yet: the reboot when a launcher cannot be killed.

Deep sleep with the heartbeat (image `home89`, real `glassd`, on battery): 39 suspends, 0 failures, 36 of them reaching `core_pwrdm` OFF. The same launcher ran throughout, the heartbeat went on from 3 to 91, the counter stayed 0, with no marker, no watchdog action and no oops.
