//! Standalone GLES2 bring-up probe for the `glass-gfx-gles` renderer.
//!
//! This is a self-contained test harness (no launcher, no glassd, no strings/setup) used to
//! bring the PowerVR SGX540 up on OpenMyGlass "Model B" (Alpine/musl, no Android) and to
//! prove the renderer on a host GL. It exercises exactly the public API the launcher uses:
//! [`GlesCompositor::offscreen`], [`GlesCompositor::for_window`] + `attach_window`, the
//! `glass-ui` timeline, and `present`.
//!
//! Modes (argv[1]):
//!
//! * `offscreen [out.png]` — create an off-screen GLES2 context, render the home timeline
//!   into a 640×360 frame, read it back and write a PNG. Runs on any EGL/GLES2 (Mesa on the
//!   host: `EGL_PLATFORM=surfaceless`). Proves the renderer end to end without a window.
//!
//! * `probe` — create a *window* GLES2 context ([`GlesCompositor::for_window`]) but attach no
//!   window: this already loads `libEGL`/`libGLESv2`, opens the display, chooses a config,
//!   creates the context and makes it current on a 1×1 pbuffer, then prints `GL_RENDERER` etc.
//!   On Model B this is the make-or-break check: it answers "does an EGL/GLES2 context come up
//!   against the PowerVR driver + pvrsrvkm?" without needing a native-window implementation.
//!
//! * `window [provider.so]` — like `probe`, then obtain a native window and present one frame
//!   to it (`eglSwapBuffers`). The native window is not created here: on a bionic/Android
//!   device without SurfaceFlinger it must come from a window-system shim (libhybris'
//!   FramebufferNativeWindow / HWComposerNativeWindow). The provider is a shared library that
//!   exports `glass_gfx_create_native_window(u32 width, u32 height, i32 format) -> *mut c_void`
//!   returning an `ANativeWindow*`; its path is argv[2] or `$GLASS_NATIVE_WINDOW_LIB`. This
//!   keeps the example decoupled from any one libhybris ABI; see docs/model-b-gpu.md.
//!
//! All diagnostics go to stderr, prefixed `gles-probe:`, so the Model B init can tee them to
//! /dev/kmsg as `OMGB-GPU:`.

use glass_gfx::{Color, Compositor, HeadlessDisplay, Placement};
use glass_gfx_gles::{GlesCompositor, GlesOptions};
use glass_ui::{Card, CardContent, RenderContext, Timeline};
use std::ffi::c_void;

const W: u32 = 640;
const H: u32 = 360;

fn ctx() -> RenderContext {
    RenderContext {
        now_s: 1_792_579_320,
        hour: 10,
        minute: 42,
        locale: glass_ui::Locale::En,
    }
}

/// The home timeline (a clock card plus one message card), as the launcher shows it.
fn timeline() -> Timeline {
    let mut t = Timeline::new(
        vec![Card::new(1, CardContent::settings("Settings"))],
        Card::new(10, CardContent::Home),
        vec![Card::new(
            11,
            CardContent::Text {
                body: "GPU bring-up frame rendered by glass-gfx-gles on the SGX540.".into(),
                source: "OpenMyGlass".into(),
                timestamp_s: ctx().now_s - 5 * 60,
            },
        )],
    );
    t.set_context(ctx());
    t
}

fn log(msg: &str) {
    eprintln!("gles-probe: {msg}");
}

fn render_home(c: &mut GlesCompositor) -> Result<(), String> {
    timeline()
        .render(c)
        .map_err(|e| format!("timeline render: {e}"))?;
    Ok(())
}

fn print_info(c: &GlesCompositor) {
    log(&format!("GL info: {}", c.info()));
    log(&format!(
        "max_texture_size={} fragment_highp={}",
        c.info().max_texture_size,
        c.info().fragment_highp
    ));
}

/// Options with the SGX540 assertion layer on (as on the device / in debug builds).
fn options() -> GlesOptions {
    GlesOptions {
        sgx540_checks: true,
        ..GlesOptions::default()
    }
}

fn offscreen(out: &str) -> Result<(), String> {
    log(&format!("offscreen: creating {W}x{H} GLES2 context"));
    let mut c = GlesCompositor::offscreen(W, H, options())
        .map_err(|e| format!("GlesCompositor::offscreen: {e}"))?;
    print_info(&c);
    render_home(&mut c)?;
    let frame = c.read_frame().map_err(|e| format!("read_frame: {e}"))?;
    frame
        .write_png(out)
        .map_err(|e| format!("write_png {out}: {e}"))?;
    log(&format!("offscreen: wrote {out} ({W}x{H})"));
    Ok(())
}

fn probe() -> Result<GlesCompositor, String> {
    log("probe: creating window GLES2 context (no window attached yet)");
    let c = GlesCompositor::for_window(W, H, options())
        .map_err(|e| format!("GlesCompositor::for_window: {e}"))?;
    print_info(&c);
    log(&format!(
        "probe: EGL/GLES2 context is up; native_visual_id={:?}",
        c.native_visual_id()
    ));
    Ok(c)
}

type CreateNativeWindow = unsafe extern "C" fn(u32, u32, i32) -> *mut c_void;

fn window(provider: Option<String>) -> Result<(), String> {
    let mut c = probe()?;
    let provider = provider
        .or_else(|| std::env::var("GLASS_NATIVE_WINDOW_LIB").ok())
        .ok_or("window: no native-window provider (argv[2] or $GLASS_NATIVE_WINDOW_LIB)")?;
    let visual = c
        .native_visual_id()
        .map_err(|e| format!("native_visual_id: {e}"))?;
    log(&format!(
        "window: loading provider {provider}, visual id {visual}"
    ));
    // SAFETY: the provider library is supplied by the operator and follows the documented ABI.
    let lib = unsafe { libloading::Library::new(&provider) }
        .map_err(|e| format!("load provider {provider}: {e}"))?;
    let win = unsafe {
        let f: libloading::Symbol<CreateNativeWindow> = lib
            .get(b"glass_gfx_create_native_window\0")
            .map_err(|e| format!("provider missing glass_gfx_create_native_window: {e}"))?;
        f(W, H, visual)
    };
    if win.is_null() {
        return Err("window: provider returned a null native window".into());
    }
    log("window: attaching native window");
    // SAFETY: the window is valid until the compositor is dropped (end of this function).
    unsafe { c.attach_window(win) }.map_err(|e| format!("attach_window: {e}"))?;
    log(&format!("window: attached, size={:?}", c.size()));
    render_home(&mut c)?;
    // present() on a window compositor calls eglSwapBuffers; the display argument is unused.
    let mut sink = HeadlessDisplay::new(W, H);
    c.present(&mut sink)
        .map_err(|e| format!("present/swap: {e}"))?;
    log("window: presented one frame (eglSwapBuffers) to the native window");
    // A second frame so a tear/!swap is visible, then detach cleanly.
    c.compose(Color::BLACK, &[] as &[Placement])
        .map_err(|e| format!("compose: {e}"))?;
    c.present(&mut sink)
        .map_err(|e| format!("present/swap 2: {e}"))?;
    c.detach_window();
    log("window: done");
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("offscreen");
    let result = match mode {
        "offscreen" => offscreen(args.get(2).map(String::as_str).unwrap_or("gles-probe.png")),
        "probe" => probe().map(|_| ()),
        "window" => window(args.get(2).cloned()),
        other => Err(format!(
            "unknown mode {other:?}; use offscreen|probe|window"
        )),
    };
    match result {
        Ok(()) => log(&format!("{mode}: OK")),
        Err(e) => {
            log(&format!("{mode}: FAIL: {e}"));
            std::process::exit(1);
        }
    }
}
