//! The native side of the Android launcher wrapper, `libglass_launcher_jni.so`.
//!
//! - [`control`]: commands from the activity to the render thread, with a wake pipe;
//! - [`runtime`]: the render thread's loop (EGL window compositor, `App`, `glassd`);
//! - `jni` (Android only): the `NativeBridge` entry points, logcat, `ANativeWindow`.
//!
//! The Java side is in `android/launcher/` (`org.openmyglass.launcher`).

pub mod control;
pub mod runtime;

#[cfg(target_os = "android")]
pub mod jni;
