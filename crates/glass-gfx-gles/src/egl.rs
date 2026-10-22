//! The EGL platform layer: loads `libEGL` and `libGLESv2` at run time, creates a GLES2
//! context and loads the GL functions.
//!
//! - EGL through `khronos-egl` as `DynamicInstance<EGL1_4>` (Android's and libhybris' libEGL
//!   are EGL 1.4); the EGL 1.5 entry points are used only when the library has them.
//! - Core GLES2 functions are looked up with `dlsym` in `libGLESv2` first;
//!   `eglGetProcAddress` only for what is missing there (extensions): before EGL 1.5 it need
//!   not return core functions.
//! - Off-screen contexts (host tests and development): Mesa's surfaceless platform when the
//!   client extensions offer it, else the default display; no surface when
//!   `EGL_KHR_surfaceless_context` is there, else a 1×1 pbuffer. Rendering goes to FBOs.

use khronos_egl as egl;
use std::ffi::{c_void, CStr};

/// The EGL library name on this platform.
pub const EGL_LIBRARY: &str = if cfg!(target_os = "android") {
    "libEGL.so"
} else {
    "libEGL.so.1"
};

/// The GLES2 library name on this platform.
pub const GLES_LIBRARY: &str = if cfg!(target_os = "android") {
    "libGLESv2.so"
} else {
    "libGLESv2.so.2"
};

/// `EGL_PLATFORM_SURFACELESS_MESA` (EGL_MESA_platform_surfaceless).
const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

type GetPlatformDisplayExt =
    unsafe extern "C" fn(egl::Enum, *mut c_void, *const egl::Int) -> egl::EGLDisplay;

/// What was set up, for logs.
#[derive(Clone, Debug)]
pub struct EglInfo {
    pub version: (i32, i32),
    pub vendor: String,
    /// "surfaceless" (Mesa platform), "default display, no surface" or "default display,
    /// pbuffer".
    pub setup: String,
}

/// A current GLES2 context with its EGL objects. Not `Send`: one context, one thread.
pub struct EglContext {
    egl: egl::DynamicInstance<egl::EGL1_4>,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    /// The 1×1 pbuffer that keeps the context current without a window (or the off-screen
    /// surface); `None` with a surfaceless context, or when the config has no pbuffers.
    surface: Option<egl::Surface>,
    /// The window surface while a native window is attached ([`EglContext::attach_window`]).
    window: Option<egl::Surface>,
    gles: libloading::Library,
    info: EglInfo,
    _not_send: std::marker::PhantomData<*const ()>,
}

fn has_ext(list: &str, name: &str) -> bool {
    list.split_ascii_whitespace().any(|e| e == name)
}

fn egl_err(what: &str, e: egl::Error) -> String {
    format!("{what}: {e}")
}

impl EglContext {
    /// An off-screen GLES2 context, made current on this thread. `force_pbuffer` skips the
    /// surfaceless platform and context and uses the default display with a pbuffer, the
    /// set-up of EGL 1.4 libraries (tests that path on Mesa).
    pub fn offscreen(force_pbuffer: bool) -> Result<EglContext, String> {
        Self::create(force_pbuffer, false)
    }

    /// A GLES2 context for rendering into a native window (Android's `ANativeWindow`):
    /// the default display and a config with `EGL_WINDOW_BIT` (with `EGL_PBUFFER_BIT` too
    /// when there is one, so a 1×1 pbuffer keeps the context current while no window is
    /// attached). The context is current on the pbuffer if there is one; attach the window
    /// with [`EglContext::attach_window`]. Call [`EglContext::native_visual_id`] first to
    /// set the window's buffer format to the config's.
    pub fn for_window() -> Result<EglContext, String> {
        Self::create(true, true)
    }

    fn create(force_pbuffer: bool, for_window: bool) -> Result<EglContext, String> {
        // SAFETY: the library is the system's EGL; its symbols follow the EGL API.
        let egl = unsafe {
            egl::DynamicInstance::<egl::EGL1_4>::load_required_from_filename(EGL_LIBRARY)
        }
        .map_err(|e| format!("cannot load {EGL_LIBRARY}: {e}"))?;
        // Client extensions (EGL 1.5 or EGL_EXT_client_extensions); an error means none.
        let client_ext = egl
            .query_string(None, egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut setup = String::new();
        let mut display = None;
        if !force_pbuffer && !for_window && has_ext(&client_ext, "EGL_MESA_platform_surfaceless") {
            display = Self::surfaceless_display(&egl, &client_ext);
            if display.is_some() {
                setup.push_str("surfaceless");
            }
        }
        let display = match display {
            Some(d) => d,
            None => {
                setup.push_str("default display");
                // SAFETY: EGL_DEFAULT_DISPLAY is always a valid native display.
                unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }
                    .ok_or("eglGetDisplay returned no display")?
            }
        };
        let (major, minor) = egl
            .initialize(display)
            .map_err(|e| egl_err("eglInitialize", e))?;
        let display_ext = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let vendor = egl
            .query_string(Some(display), egl::VENDOR)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let surfaceless_ctx =
            !force_pbuffer && has_ext(&display_ext, "EGL_KHR_surfaceless_context");
        let attribs = |surface_type| {
            [
                egl::RED_SIZE,
                8,
                egl::GREEN_SIZE,
                8,
                egl::BLUE_SIZE,
                8,
                egl::RENDERABLE_TYPE,
                egl::OPENGL_ES2_BIT,
                egl::SURFACE_TYPE,
                surface_type,
                egl::NONE,
            ]
        };
        let choose = |surface_type| {
            egl.choose_first_config(display, &attribs(surface_type))
                .map_err(|e| egl_err("eglChooseConfig", e))
        };
        // (config, whether it has pbuffers)
        let (config, pbuffers) = if for_window {
            match choose(egl::WINDOW_BIT | egl::PBUFFER_BIT)? {
                Some(c) => (c, true),
                None => (
                    choose(egl::WINDOW_BIT)?
                        .ok_or("no EGL window config for GLES2 with 8-bit colour")?,
                    false,
                ),
            }
        } else {
            let surface_type = if surfaceless_ctx { 0 } else { egl::PBUFFER_BIT };
            (
                choose(surface_type)?.ok_or("no EGL config for GLES2 with 8-bit colour")?,
                !surfaceless_ctx,
            )
        };
        egl.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| egl_err("eglBindAPI", e))?;
        let context = egl
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
            )
            .map_err(|e| egl_err("eglCreateContext", e))?;
        let mut surface = None;
        let rest = (|| {
            if for_window {
                setup.push_str(", window");
            }
            if surfaceless_ctx || (for_window && !pbuffers) {
                setup.push_str(", no surface");
            } else {
                setup.push_str(", pbuffer");
                surface = Some(
                    egl.create_pbuffer_surface(
                        display,
                        config,
                        &[egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE],
                    )
                    .map_err(|e| egl_err("eglCreatePbufferSurface", e))?,
                );
            }
            // A window context without pbuffers becomes current with its first window.
            if !(for_window && !pbuffers && !surfaceless_ctx) {
                egl.make_current(display, surface, surface, Some(context))
                    .map_err(|e| egl_err("eglMakeCurrent", e))?;
            }
            // SAFETY: the system's GLES2 library.
            unsafe { libloading::Library::new(GLES_LIBRARY) }
                .map_err(|e| format!("cannot load {GLES_LIBRARY}: {e}"))
        })();
        let gles = match rest {
            Ok(g) => g,
            Err(e) => {
                // Do not leak the context and surface of a failed set-up.
                let _ = egl.make_current(display, None, None, None);
                if let Some(s) = surface {
                    let _ = egl.destroy_surface(display, s);
                }
                let _ = egl.destroy_context(display, context);
                return Err(e);
            }
        };
        Ok(EglContext {
            egl,
            display,
            config,
            context,
            surface,
            window: None,
            gles,
            info: EglInfo {
                version: (major, minor),
                vendor,
                setup,
            },
            _not_send: std::marker::PhantomData,
        })
    }

    fn surfaceless_display(
        egl: &egl::DynamicInstance<egl::EGL1_4>,
        client_ext: &str,
    ) -> Option<egl::Display> {
        // EGL 1.5: eglGetPlatformDisplay.
        if let Some(egl15) = egl.upcast::<egl::EGL1_5>() {
            // SAFETY: EGL_DEFAULT_DISPLAY is the native display the surfaceless platform takes.
            let d = unsafe {
                egl15.get_platform_display(
                    PLATFORM_SURFACELESS_MESA,
                    egl::DEFAULT_DISPLAY,
                    &[egl::ATTRIB_NONE],
                )
            };
            if let Ok(d) = d {
                return Some(d);
            }
        }
        // EGL 1.4 with EGL_EXT_platform_base: eglGetPlatformDisplayEXT.
        if has_ext(client_ext, "EGL_EXT_platform_base") {
            let f = egl.get_proc_address("eglGetPlatformDisplayEXT")?;
            // SAFETY: the extension defines this signature.
            let f: GetPlatformDisplayExt = unsafe { std::mem::transmute(f) };
            let none = [egl::NONE];
            let ptr = unsafe {
                f(
                    PLATFORM_SURFACELESS_MESA,
                    egl::DEFAULT_DISPLAY,
                    none.as_ptr(),
                )
            };
            if !ptr.is_null() {
                // SAFETY: a display returned by EGL.
                return Some(unsafe { egl::Display::from_ptr(ptr) });
            }
        }
        None
    }

    pub fn info(&self) -> &EglInfo {
        &self.info
    }

    /// Makes the context current on this thread (several contexts can live on one thread,
    /// in tests), on the window if one is attached, else on the pbuffer (or no surface).
    pub fn make_current(&self) -> Result<(), String> {
        let s = self.window.or(self.surface);
        self.egl
            .make_current(self.display, s, s, Some(self.context))
            .map_err(|e| egl_err("eglMakeCurrent", e))
    }

    /// The config's `EGL_NATIVE_VISUAL_ID`: on Android the window's buffer format to set
    /// (`ANativeWindow_setBuffersGeometry`) before [`EglContext::attach_window`].
    pub fn native_visual_id(&self) -> Result<i32, String> {
        self.egl
            .get_config_attrib(self.display, self.config, egl::NATIVE_VISUAL_ID)
            .map_err(|e| egl_err("eglGetConfigAttrib(EGL_NATIVE_VISUAL_ID)", e))
    }

    /// Whether a window surface is attached.
    pub fn has_window(&self) -> bool {
        self.window.is_some()
    }

    /// Creates a window surface for `window` and makes the context current on it. Any
    /// window attached before is detached first.
    ///
    /// # Safety
    /// `window` must be a valid native window (an `ANativeWindow*` on Android) that stays
    /// valid until [`EglContext::detach_window`] (or the drop of this context) returns.
    pub unsafe fn attach_window(&mut self, window: *mut c_void) -> Result<(), String> {
        self.detach_window();
        // SAFETY: the caller guarantees the window; the pointer type differs per platform.
        let s = unsafe {
            self.egl.create_window_surface(
                self.display,
                self.config,
                window as egl::NativeWindowType,
                None,
            )
        }
        .map_err(|e| egl_err("eglCreateWindowSurface", e))?;
        self.window = Some(s);
        if let Err(e) = self.make_current() {
            self.detach_window();
            return Err(e);
        }
        Ok(())
    }

    /// Releases the window surface: the context stays, current on the pbuffer if there is
    /// one (else on no surface: GL calls fail until the next window). After this returns
    /// the native window is no longer used.
    pub fn detach_window(&mut self) {
        let Some(w) = self.window.take() else {
            return;
        };
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_surface(self.display, w);
        if self.surface.is_some() {
            let _ = self.make_current();
        }
    }

    /// The window surface's size in pixels, if one is attached.
    pub fn window_size(&self) -> Option<(u32, u32)> {
        let w = self.window?;
        let q = |a| self.egl.query_surface(self.display, w, a).ok();
        Some((q(egl::WIDTH)? as u32, q(egl::HEIGHT)? as u32))
    }

    /// Shows the window's back buffer (`eglSwapBuffers`).
    pub fn swap_buffers(&self) -> Result<(), String> {
        let w = self.window.ok_or("no window attached")?;
        self.egl
            .swap_buffers(self.display, w)
            .map_err(|e| egl_err("eglSwapBuffers", e))
    }

    /// Whether this context is the current one.
    pub fn is_current(&self) -> bool {
        self.egl
            .get_current_context()
            .is_some_and(|c| c == self.context)
    }

    /// Loads the GL functions: `libGLESv2` first, then `eglGetProcAddress`.
    pub fn load_gl(&self) -> glow::Context {
        let loader = |name: &CStr| -> *const c_void {
            // SAFETY: a data-free lookup of a function address.
            let sym = unsafe {
                self.gles
                    .get::<*const c_void>(name.to_bytes_with_nul())
                    .map(|s| *s)
            };
            match sym {
                Ok(p) if !p.is_null() => p,
                _ => name
                    .to_str()
                    .ok()
                    .and_then(|n| self.egl.get_proc_address(n))
                    .map_or(std::ptr::null(), |f| f as *const c_void),
            }
        };
        // SAFETY: the context is current and the loader returns GL entry points.
        unsafe { glow::Context::from_loader_function_cstr(loader) }
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        // The display is not terminated: other contexts of this process may share it.
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        if let Some(w) = self.window.take() {
            let _ = self.egl.destroy_surface(self.display, w);
        }
        if let Some(s) = self.surface {
            let _ = self.egl.destroy_surface(self.display, s);
        }
    }
}
