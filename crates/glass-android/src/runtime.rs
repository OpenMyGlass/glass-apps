//! The render thread: owns the EGL context (one context, one thread), the launcher [`App`]
//! and the `glassd` session.
//!
//! Its loop never spins: it drains the commands from the activity and the frames from
//! `glassd`, steps and draws, then blocks in `poll` on the command pipe and the `glassd`
//! socket with the timeout of [`wait_time`] (a frame while something moves, the next
//! minute for the home clock at rest, no limit while the screen is off, paused, or without
//! a surface), or until the next reconnection attempt.
//!
//! Independent of Android except through [`Native`], so the loop runs in host tests with
//! the software compositor and glass-api's mock of `glassd`.

use crate::control::{wait, Command, Inbox, WindowPtr};
use glass_gfx::{Compositor, HeadlessDisplay, GLASS_HEIGHT, GLASS_WIDTH};
use glass_launcher::glassd::{wait_time, Backoff, Session};
use glass_launcher::platform::{Platform, PlatformEvent};
use glass_launcher::{clock, demo, App};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The name of the marker file (in the app's files directory) that says setup was done.
pub const SETUP_MARKER: &str = "setup-done";

/// What the loop needs from the operating system.
pub trait Native {
    /// Logs a line (Android: logcat, tag `glass-launcher`).
    fn log(&self, error: bool, msg: &str);
    /// Sets the window's buffers to `width`×`height` in `format`
    /// (`ANativeWindow_setBuffersGeometry`); returns its result (0 on success).
    fn set_buffers_geometry(&self, window: WindowPtr, width: i32, height: i32, format: i32) -> i32;
    /// Drops the render thread's reference to the window (`ANativeWindow_release`).
    fn release_window(&self, window: WindowPtr);
}

/// Settings of [`run`].
pub struct Config {
    pub socket: PathBuf,
    pub files_dir: PathBuf,
    /// Frame rate while something moves.
    pub fps: u32,
    pub backoff: Backoff,
}

impl Config {
    pub fn new(files_dir: PathBuf) -> Config {
        Config {
            socket: glass_api::socket_path(),
            files_dir,
            fps: 60,
            backoff: Backoff::default(),
        }
    }
}

/// Without a renderer nothing is drawn, but the activity's commands are still answered
/// (a surface must be released, `Quit` ends the thread).
fn idle(inbox: &Inbox, native: &dyn Native) {
    loop {
        wait(inbox, None, None);
        for c in inbox.drain() {
            match c {
                Command::SurfaceCreated(w) => native.release_window(w),
                Command::SurfaceDestroyed(ack) => {
                    let _ = ack.send(());
                }
                Command::Quit => return,
                _ => {}
            }
        }
    }
}

/// Runs the launcher until [`Command::Quit`]. `compositor` is the renderer that was set up
/// (on the device: the GLES window compositor; its failure is a hard error and nothing is
/// drawn, there is no software fallback).
pub fn run(
    cfg: Config,
    inbox: Inbox,
    native: &dyn Native,
    compositor: Result<Box<dyn Compositor>, String>,
    platform: Box<dyn Platform>,
) {
    let comp = match compositor {
        Ok(c) => c,
        Err(e) => {
            native.log(
                true,
                &format!("GLES renderer failed: {e}; nothing is drawn (no software fallback)"),
            );
            return idle(&inbox, native);
        }
    };
    native.log(false, &format!("renderer {}", comp.name()));
    let ctx = clock::now();
    let timeline = demo::device_timeline(ctx.now_s, platform.name(), comp.name());
    let mut app = App::with_compositor(timeline, ctx, comp).with_platform(platform);
    app.follow_glassd();
    if app.start_setup(Some(cfg.files_dir.join(SETUP_MARKER))) {
        native.log(false, "setup flow started");
    }
    let mut window: Option<WindowPtr> = None;
    let mut resumed = false;
    let mut session: Option<Session> = None;
    let mut backoff = cfg.backoff.clone();
    let mut retry_at = Instant::now();
    let mut said_unavailable = false;
    // Off-screen compositors (host tests) present into this; a window compositor swaps.
    let mut display = HeadlessDisplay::new(GLASS_WIDTH, GLASS_HEIGHT);
    let mut last = Instant::now();
    loop {
        for cmd in inbox.drain() {
            match cmd {
                Command::SurfaceCreated(w) => {
                    if let Some(old) = window.take() {
                        if let Some(t) = app.window_target() {
                            t.detach_window();
                        }
                        native.release_window(old);
                    }
                    match app.window_target() {
                        Some(t) => {
                            let format = t.native_visual_id().unwrap_or(0);
                            let r = native.set_buffers_geometry(
                                w,
                                GLASS_WIDTH as i32,
                                GLASS_HEIGHT as i32,
                                format,
                            );
                            if r != 0 {
                                native.log(true, &format!("setBuffersGeometry: {r}"));
                            }
                            // SAFETY: the window holds a reference for us until
                            // `release_window`, which comes only after `detach_window`.
                            match unsafe { t.attach_window(w.0) } {
                                Ok(()) => window = Some(w),
                                Err(e) => {
                                    native.log(true, &format!("window surface: {e}"));
                                    native.release_window(w);
                                }
                            }
                        }
                        None => native.release_window(w),
                    }
                    app.invalidate();
                }
                Command::SurfaceChanged { .. } => app.invalidate(),
                Command::SurfaceDestroyed(ack) => {
                    if let Some(w) = window.take() {
                        if let Some(t) = app.window_target() {
                            t.detach_window();
                        }
                        native.release_window(w);
                    }
                    let _ = ack.send(());
                }
                Command::Resume | Command::Pause => {
                    resumed = matches!(cmd, Command::Resume);
                    if let Some(s) = session.as_mut() {
                        if let Err(e) = s.set_wants_focus(resumed, &mut app) {
                            native.log(true, &format!("glassd: {e}"));
                            app.glassd_disconnected();
                            session = None;
                        }
                    }
                    app.invalidate();
                }
                Command::Event(line) => match PlatformEvent::parse(&line) {
                    Some(e) => app.platform_event(&e),
                    None => native.log(true, &format!("unknown platform event {line:?}")),
                },
                Command::Quit => {
                    if let Some(w) = window.take() {
                        if let Some(t) = app.window_target() {
                            t.detach_window();
                        }
                        native.release_window(w);
                    }
                    if let Some(s) = session.as_mut() {
                        let _ = s.set_wants_focus(false, &mut app);
                    }
                    return;
                }
            }
        }
        if session.is_none() && Instant::now() >= retry_at {
            match Session::connect_with(&cfg.socket, resumed, true) {
                Ok(s) => {
                    backoff.reset();
                    said_unavailable = false;
                    app.cancel_touch();
                    app.set_service(Some(s.service_info()));
                    native.log(
                        false,
                        &format!("connected to glassd at {}", cfg.socket.display()),
                    );
                    session = Some(s);
                }
                Err(e) => {
                    let d = backoff.next_delay();
                    retry_at = Instant::now() + d;
                    if !said_unavailable {
                        native.log(true, &format!("glassd unavailable ({e}); retrying"));
                        said_unavailable = true;
                    }
                }
            }
        }
        if let Some(s) = session.as_mut() {
            let mut lost = None;
            loop {
                match s.pump(&mut app, Some(Duration::from_micros(1))) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) => {
                        lost = Some(e);
                        break;
                    }
                }
            }
            for l in s.take_log() {
                native.log(l.contains("ERROR_CODE_"), &l);
            }
            if let Some(e) = lost {
                native.log(true, &format!("glassd connection lost ({e}); reconnecting"));
                app.glassd_disconnected();
                session = None;
                retry_at = Instant::now();
                continue;
            }
        }
        for l in app.take_log() {
            native.log(false, &l);
        }
        let now = Instant::now();
        app.step((now - last).as_secs_f32(), clock::now());
        last = now;
        let has_window = window.is_some() || app.window_target().is_none();
        let can_draw = resumed && has_window;
        if can_draw {
            if let Err(e) = app.present_if_changed(&mut display) {
                native.log(true, &format!("render: {e}"));
            }
        }
        // Requests the drawing made (none today) go out with the next pump.
        let mut timeout = if can_draw {
            wait_time(&app, clock::now().now_s, cfg.fps, None)
        } else {
            None
        };
        if session.is_none() {
            let left = retry_at.saturating_duration_since(Instant::now());
            timeout = Some(timeout.map_or(left, |t| t.min(left)));
        }
        if let Some(s) = session.as_mut() {
            // Requests made by input or events since the last pump.
            if let Err(e) = s.send_requests(&mut app) {
                native.log(true, &format!("glassd: {e}"));
            }
        }
        wait(&inbox, session.as_ref().map(|s| s.raw_fd()), timeout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::channel;
    use glass_api::MockService;
    use glass_gfx::SoftwareCompositor;
    use glass_launcher::glassd::CLIENT_NAME;
    use glass_launcher::platform::MockPlatform;
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestNative {
        lines: Mutex<Vec<String>>,
    }

    impl Native for TestNative {
        fn log(&self, _error: bool, msg: &str) {
            self.lines.lock().unwrap().push(msg.to_string());
        }
        fn set_buffers_geometry(&self, _: WindowPtr, _: i32, _: i32, _: i32) -> i32 {
            0
        }
        fn release_window(&self, _: WindowPtr) {
            self.lines.lock().unwrap().push("released".into());
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("glass-android-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn focus_follows_resume_and_pause_and_quit_ends() {
        let m = MockService::start().unwrap();
        let files = dir("focus");
        let (control, inbox) = channel().unwrap();
        let mut cfg = Config::new(files.clone());
        cfg.socket = m.path().to_path_buf();
        let native = std::sync::Arc::new(TestNative::default());
        let n2 = native.clone();
        let h = std::thread::spawn(move || {
            run(
                cfg,
                inbox,
                &*n2,
                Ok(Box::new(SoftwareCompositor::glass())),
                Box::new(MockPlatform::default()),
            )
        });
        let launcher_focused = |m: &MockService| {
            m.wait_until(T, |h| {
                h.focus_holder()
                    .is_some_and(|id| h.client_name(id) == Some(CLIENT_NAME))
            })
        };
        // Connected but paused: no focus.
        assert!(m.wait_until(T, |h| h
            .clients()
            .any(|id| h.client_name(id) == Some(CLIENT_NAME))));
        assert!(m.with_hub(|h| h.focus_holder().is_none()));
        control.send(Command::Resume);
        assert!(launcher_focused(&m));
        control.send(Command::Pause);
        assert!(m.wait_until(T, |h| h.focus_holder().is_none()));
        // Already set up elsewhere: the marker is written.
        control.send(Command::Event("provisioned\ttrue".into()));
        control.send(Command::Event("garbage".into()));
        // A surface on an off-screen compositor is released at once.
        control.send(Command::SurfaceCreated(WindowPtr(std::ptr::null_mut())));
        assert!(control.surface_destroyed(T));
        control.send(Command::Quit);
        h.join().unwrap();
        assert!(files.join(SETUP_MARKER).exists());
        let lines = native.lines.lock().unwrap().clone();
        assert!(
            lines.iter().any(|l| l.contains("unknown platform event")),
            "{lines:#?}"
        );
        assert!(lines.iter().any(|l| l == "released"), "{lines:#?}");
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn a_failed_renderer_draws_nothing_but_answers() {
        let (control, inbox) = channel().unwrap();
        let native = std::sync::Arc::new(TestNative::default());
        let n2 = native.clone();
        let cfg = Config::new(dir("fail"));
        let h = std::thread::spawn(move || {
            run(
                cfg,
                inbox,
                &*n2,
                Err("no EGL".into()),
                Box::new(MockPlatform::default()),
            )
        });
        assert!(control.surface_destroyed(T));
        control.send(Command::Quit);
        h.join().unwrap();
        let lines = native.lines.lock().unwrap().clone();
        assert!(
            lines[0].starts_with("GLES renderer failed: no EGL"),
            "{lines:#?}"
        );
    }

    #[test]
    fn reconnects_when_glassd_comes_up_later() {
        let files = dir("later");
        let sock = files.join("glassd.sock");
        let (control, inbox) = channel().unwrap();
        let mut cfg = Config::new(files.clone());
        cfg.socket = sock.clone();
        cfg.backoff = Backoff::new(Duration::from_millis(10), Duration::from_millis(40));
        let native = std::sync::Arc::new(TestNative::default());
        let n2 = native.clone();
        let h = std::thread::spawn(move || {
            run(
                cfg,
                inbox,
                &*n2,
                Ok(Box::new(SoftwareCompositor::glass())),
                Box::new(MockPlatform::default()),
            )
        });
        control.send(Command::Resume);
        std::thread::sleep(Duration::from_millis(100));
        let m = MockService::start().unwrap();
        std::os::unix::fs::symlink(m.path(), &sock).unwrap();
        assert!(m.wait_until(T, |h| {
            h.focus_holder()
                .is_some_and(|id| h.client_name(id) == Some(CLIENT_NAME))
        }));
        control.send(Command::Quit);
        h.join().unwrap();
        let _ = std::fs::remove_dir_all(&files);
    }
}
