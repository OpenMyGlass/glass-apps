//! The JNI entry points of `org.openmyglass.launcher.NativeBridge` and the Android side of
//! [`Native`]: logcat, `ANativeWindow`, and the one call back into Java,
//! `NativeBridge.onNativeRequest(String)` (platform request lines, made on the render
//! thread, which is attached to the VM once).
//!
//! The surface is minimal and string based (Java → native):
//! `nativeCreate(String filesDir) -> long`, `nativeSurfaceCreated(long, Surface)`,
//! `nativeSurfaceChanged(long, int, int)`, `nativeSurfaceDestroyed(long)` (blocks until the
//! render thread let the window go), `nativeResume(long)`, `nativePause(long)`,
//! `nativeEvent(long, String)`, `nativeDestroy(long)`.
//!
//! Safety of the entry points: they are called only by the VM with a valid `JNIEnv`, and
//! the `long` handle is the one `nativeCreate` returned, until `nativeDestroy`.
#![allow(clippy::missing_safety_doc)]

use crate::control::{channel, Command, Control, WindowPtr};
use crate::runtime::{self, Config, Native};
use glass_gfx::{Compositor, GLASS_HEIGHT, GLASS_WIDTH};
use glass_launcher::platform::LinePlatform;
use jni_sys::{jlong, jobject, jstring, jvalue, JNIEnv, JavaVM, JNI_OK};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Duration;

const TAG: &CStr = c"glass-launcher";
const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_ERROR: c_int = 6;

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

#[link(name = "android")]
extern "C" {
    fn ANativeWindow_fromSurface(env: *mut JNIEnv, surface: jobject) -> *mut c_void;
    fn ANativeWindow_release(window: *mut c_void);
    fn ANativeWindow_setBuffersGeometry(
        window: *mut c_void,
        width: i32,
        height: i32,
        format: i32,
    ) -> i32;
}

/// Writes a line to logcat.
pub fn log(error: bool, msg: &str) {
    let text = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    let prio = if error {
        ANDROID_LOG_ERROR
    } else {
        ANDROID_LOG_INFO
    };
    // SAFETY: both strings are NUL-terminated and live for the call.
    unsafe { __android_log_write(prio, TAG.as_ptr(), text.as_ptr()) };
}

struct AndroidNative;

impl Native for AndroidNative {
    fn log(&self, error: bool, msg: &str) {
        log(error, msg)
    }

    fn set_buffers_geometry(&self, w: WindowPtr, width: i32, height: i32, format: i32) -> i32 {
        // SAFETY: the render thread holds a reference to the window.
        unsafe { ANativeWindow_setBuffersGeometry(w.0, width, height, format) }
    }

    fn release_window(&self, w: WindowPtr) {
        if !w.0.is_null() {
            // SAFETY: releases the reference taken by ANativeWindow_fromSurface.
            unsafe { ANativeWindow_release(w.0) };
        }
    }
}

/// The Java bridge object, called back from the render thread.
struct JavaBridge {
    vm: *mut JavaVM,
    bridge: jobject,
    env: *mut JNIEnv,
}

// SAFETY: a JavaVM and a global reference may be used from any attached thread; the
// bridge moves to the render thread once and is only used there.
unsafe impl Send for JavaBridge {}

impl JavaBridge {
    /// Attaches this thread on first use.
    unsafe fn env(&mut self) -> Option<*mut JNIEnv> {
        if self.env.is_null() {
            let mut env: *mut c_void = std::ptr::null_mut();
            let attach = (**self.vm).AttachCurrentThread?;
            if attach(self.vm, &mut env, std::ptr::null_mut()) != JNI_OK {
                return None;
            }
            self.env = env.cast();
        }
        Some(self.env)
    }

    /// `bridge.onNativeRequest(line)`.
    fn request(&mut self, line: &str) -> Result<(), String> {
        // SAFETY: standard JNI calls on an attached thread with a live global reference.
        unsafe {
            let env = self
                .env()
                .ok_or("cannot attach the render thread to the VM")?;
            let f = &**env;
            let cls = (f.GetObjectClass.unwrap())(env, self.bridge);
            let mid = (f.GetMethodID.unwrap())(
                env,
                cls,
                c"onNativeRequest".as_ptr(),
                c"(Ljava/lang/String;)V".as_ptr(),
            );
            (f.DeleteLocalRef.unwrap())(env, cls);
            if mid.is_null() {
                (f.ExceptionClear.unwrap())(env);
                return Err("NativeBridge.onNativeRequest is missing".into());
            }
            let c = CString::new(line.replace('\0', " ")).unwrap_or_default();
            let s = (f.NewStringUTF.unwrap())(env, c.as_ptr());
            let args = [jvalue { l: s }];
            (f.CallVoidMethodA.unwrap())(env, self.bridge, mid, args.as_ptr());
            (f.DeleteLocalRef.unwrap())(env, s);
            if (f.ExceptionCheck.unwrap())(env) != 0 {
                (f.ExceptionDescribe.unwrap())(env);
                (f.ExceptionClear.unwrap())(env);
                return Err(format!("onNativeRequest({line}) threw"));
            }
        }
        Ok(())
    }
}

impl Drop for JavaBridge {
    fn drop(&mut self) {
        // SAFETY: drop the global reference and detach the render thread.
        unsafe {
            if let Some(env) = self.env() {
                ((**env).DeleteGlobalRef.unwrap())(env, self.bridge);
            }
            if !self.env.is_null() {
                if let Some(detach) = (**self.vm).DetachCurrentThread {
                    detach(self.vm);
                }
            }
        }
    }
}

struct Handle {
    control: Control,
    thread: Option<JoinHandle<()>>,
}

unsafe fn string(env: *mut JNIEnv, s: jstring) -> String {
    if s.is_null() {
        return String::new();
    }
    let f = &**env;
    let p = (f.GetStringUTFChars.unwrap())(env, s, std::ptr::null_mut());
    if p.is_null() {
        return String::new();
    }
    let out = CStr::from_ptr(p).to_string_lossy().into_owned();
    (f.ReleaseStringUTFChars.unwrap())(env, s, p);
    out
}

unsafe fn handle<'a>(h: jlong) -> Option<&'a Handle> {
    (h as *const Handle).as_ref()
}

/// The GLES window compositor; its failure is returned (the render thread then logs it as
/// a hard error and draws nothing).
fn gles() -> Result<Box<dyn Compositor>, String> {
    let options = glass_gfx_gles::GlesOptions {
        // The SGX540 assertion layer costs little; keep it in debug builds only.
        ..Default::default()
    };
    let c = glass_gfx_gles::GlesCompositor::for_window(GLASS_WIDTH, GLASS_HEIGHT, options)
        .map_err(|e| e.to_string())?;
    log(false, &format!("gles: {}", c.info()));
    Ok(Box::new(c))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_openmyglass_launcher_NativeBridge_nativeCreate(
    env: *mut JNIEnv,
    this: jobject,
    files_dir: jstring,
) -> jlong {
    let f = &**env;
    let dir = PathBuf::from(string(env, files_dir));
    let mut vm: *mut JavaVM = std::ptr::null_mut();
    if (f.GetJavaVM.unwrap())(env, &mut vm) != JNI_OK {
        log(true, "GetJavaVM failed");
        return 0;
    }
    let bridge = (f.NewGlobalRef.unwrap())(env, this);
    let mut java = JavaBridge {
        vm,
        bridge,
        env: std::ptr::null_mut(),
    };
    let (control, inbox) = match channel() {
        Ok(c) => c,
        Err(e) => {
            log(true, &format!("pipe: {e}"));
            return 0;
        }
    };
    let thread = std::thread::Builder::new()
        .name("glass-render".into())
        .spawn(move || {
            // EGL and the GL context are created on this thread, which then owns them.
            let compositor = gles();
            let platform =
                LinePlatform::new("android", Box::new(move |line: &str| java.request(line)));
            runtime::run(
                Config::new(dir),
                inbox,
                &AndroidNative,
                compositor,
                Box::new(platform),
            );
        });
    let thread = match thread {
        Ok(t) => t,
        Err(e) => {
            log(true, &format!("render thread: {e}"));
            return 0;
        }
    };
    Box::into_raw(Box::new(Handle {
        control,
        thread: Some(thread),
    })) as jlong
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_openmyglass_launcher_NativeBridge_nativeSurfaceCreated(
    env: *mut JNIEnv,
    _this: jobject,
    h: jlong,
    surface: jobject,
) {
    let Some(h) = handle(h) else { return };
    let w = ANativeWindow_fromSurface(env, surface);
    if w.is_null() {
        log(true, "ANativeWindow_fromSurface returned null");
        return;
    }
    if !h.control.send(Command::SurfaceCreated(WindowPtr(w))) {
        ANativeWindow_release(w);
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_openmyglass_launcher_NativeBridge_nativeSurfaceChanged(
    _env: *mut JNIEnv,
    _this: jobject,
    h: jlong,
    width: i32,
    height: i32,
) {
    if let Some(h) = handle(h) {
        h.control.send(Command::SurfaceChanged { width, height });
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_openmyglass_launcher_NativeBridge_nativeSurfaceDestroyed(
    _env: *mut JNIEnv,
    _this: jobject,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        // Below the input dispatch timeout, so a stuck render thread is logged, not an ANR.
        if !h.control.surface_destroyed(Duration::from_secs(3)) {
            log(
                true,
                "the render thread did not release the surface in time",
            );
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_openmyglass_launcher_NativeBridge_nativeResume(
    _env: *mut JNIEnv,
    _this: jobject,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        h.control.send(Command::Resume);
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_openmyglass_launcher_NativeBridge_nativePause(
    _env: *mut JNIEnv,
    _this: jobject,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        h.control.send(Command::Pause);
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_openmyglass_launcher_NativeBridge_nativeEvent(
    env: *mut JNIEnv,
    _this: jobject,
    h: jlong,
    line: jstring,
) {
    if let Some(h) = handle(h) {
        h.control.send(Command::Event(string(env, line)));
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_openmyglass_launcher_NativeBridge_nativeDestroy(
    _env: *mut JNIEnv,
    _this: jobject,
    h: jlong,
) {
    if h == 0 {
        return;
    }
    let mut h = Box::from_raw(h as *mut Handle);
    h.control.send(Command::Quit);
    if let Some(t) = h.thread.take() {
        let _ = t.join();
    }
}
