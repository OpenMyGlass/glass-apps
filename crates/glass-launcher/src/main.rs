//! `glass-launcher`: the card timeline launcher.
//!
//! ```text
//! glass-launcher --headless --script FILE --out DIR [--fps N] [--time HH:MM] [--now UNIX_S]
//!                [--renderer gles|software]
//! glass-launcher [--window] [--time HH:MM] [--renderer gles|software]   (needs `window`)
//! glass-launcher --glassd [SOCKET] [--out DIR | --window] [--time HH:MM] [--renderer R]
//! ```

use glass_gfx::HeadlessDisplay;
use glass_launcher::renderer::{compositor_for, Renderer};
use glass_launcher::script::{parse, Runner};
use glass_launcher::{clock, demo, glassd, App};
use glass_ui::RenderContext;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage:
  glass-launcher --headless --script FILE --out DIR [--fps N] [--time HH:MM] [--now UNIX_S]
                 [--renderer gles|software]
  glass-launcher [--window] [--time HH:MM] [--renderer gles|software]   (needs the `window` feature)
  glass-launcher --glassd [SOCKET] [--out DIR | --window] [--time HH:MM] [--renderer R]

  --headless      run a mock input script and write the frames as PNG files
  --script FILE   commands in the glassd --mock syntax (tap, swipe forward, scroll 0.3, wait 200ms, ...)
  --out DIR       where frame-NNNNN.png files go (created if missing)
  --fps N         frame rate of the simulation (default 30 headless, 60 in a window)
  --time HH:MM    fixed clock for the home card (default: the local time)
  --now UNIX_S    fixed 'now' for relative times (default: the demo time with --time, else now)
  --window        open a 640x360 host window: Left/Right = swipe backward/forward,
                  Enter/Space = tap, Down = swipe down, Up = swipe up, Esc = quit
  --glassd [SOCK] take input and the screen state from glassd (glass-services); SOCKET
                  defaults to $GLASS_SERVICES_SOCKET, else the platform's glassd socket.
                  Frames go to --out DIR as PNG files, to a --window, or nowhere (memory).
                  Reconnects when glassd restarts
  --renderer R    gles (GPU, the default) or software (debugging aid). On the device a
                  failing EGL is an error; on a host without EGL the default is software";

#[derive(Default)]
struct Args {
    headless: bool,
    window: bool,
    script: Option<PathBuf>,
    out: Option<PathBuf>,
    fps: Option<u32>,
    time: Option<(u8, u8)>,
    now: Option<i64>,
    renderer: Option<Renderer>,
    /// `--glassd`, with the socket if one was given.
    glassd: Option<Option<PathBuf>>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        if arg == "--glassd" {
            let sock = it.next_if(|v| !v.starts_with("--")).map(PathBuf::from);
            a.glassd = Some(sock);
            continue;
        }
        let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--headless" => a.headless = true,
            "--window" => a.window = true,
            "--script" => a.script = Some(value("--script")?.into()),
            "--out" => a.out = Some(value("--out")?.into()),
            "--fps" => {
                let v = value("--fps")?;
                a.fps = Some(
                    v.parse()
                        .ok()
                        .filter(|f| (1..=240).contains(f))
                        .ok_or("--fps must be 1..240")?,
                )
            }
            "--time" => a.time = Some(clock::parse_hh_mm(&value("--time")?).ok_or("bad --time")?),
            "--now" => a.now = Some(value("--now")?.parse().map_err(|_| "bad --now")?),
            "--renderer" => a.renderer = Some(value("--renderer")?.parse()?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.glassd.is_some() && (a.headless || a.script.is_some()) {
        return Err("--glassd excludes --headless and --script".into());
    }
    if a.glassd.is_some() && a.window && a.out.is_some() {
        return Err("--glassd takes --out or --window, not both".into());
    }
    if a.headless && a.window {
        return Err("--headless and --window exclude each other".into());
    }
    if a.headless && a.script.is_none() {
        return Err("--headless needs --script".into());
    }
    if a.headless && a.out.is_none() {
        return Err("--headless needs --out".into());
    }
    Ok(a)
}

/// The clock for a run: fixed when `--time` or `--now` is given, else the system clock.
fn clock_for(args: &Args) -> impl Fn(f32) -> RenderContext {
    let fixed = (args.time.is_some() || args.now.is_some()).then(|| {
        let real = clock::now();
        let (h, m) = args.time.unwrap_or((real.hour, real.minute));
        clock::fixed(args.now.unwrap_or(demo::DEMO_NOW_S), h, m)
    });
    move |_elapsed_s| fixed.unwrap_or_else(clock::now)
}

fn run_headless(args: &Args) -> Result<(), String> {
    let script_path = args.script.as_ref().ok_or("--headless needs --script")?;
    let out = args.out.as_ref().ok_or("--headless needs --out")?;
    let text = std::fs::read_to_string(script_path)
        .map_err(|e| format!("{}: {e}", script_path.display()))?;
    let commands = parse(&text)?;
    let clock = clock_for(args);
    let ctx = clock(0.0);
    let compositor = compositor_for(args.renderer)?;
    let mut app = App::with_compositor(demo::timeline_at(ctx.now_s), ctx, compositor);
    let mut display = HeadlessDisplay::new(glass_gfx::GLASS_WIDTH, glass_gfx::GLASS_HEIGHT)
        .with_output_dir(out)
        .map_err(|e| e.to_string())?;
    let mut runner = Runner::new(args.fps.unwrap_or(30));
    runner
        .run(&commands, &mut app, &mut display, clock)
        .map_err(|e| e.to_string())?;
    for line in runner.log() {
        eprintln!("{line}");
    }
    eprintln!(
        "{} frames written to {}",
        display.presented(),
        out.display()
    );
    Ok(())
}

#[cfg(feature = "window")]
fn run_window(args: &Args) -> Result<(), String> {
    use glass_gfx::window::{Key, WindowDisplay};
    use glass_ui::{Action, GestureKind, InputEvent};
    use std::time::{Duration, Instant};

    let clock = clock_for(args);
    let ctx = clock(0.0);
    let compositor = compositor_for(args.renderer)?;
    let mut app = App::with_compositor(demo::timeline_at(ctx.now_s), ctx, compositor);
    let mut display = WindowDisplay::new(
        "glass-launcher",
        glass_gfx::GLASS_WIDTH,
        glass_gfx::GLASS_HEIGHT,
    )
    .map_err(|e| e.to_string())?;
    let frame_time = Duration::from_micros(1_000_000 / args.fps.unwrap_or(60) as u64);
    let mut last = Instant::now();
    let start = last;
    while display.is_open() {
        for key in display.keys_pressed() {
            let kind = match key {
                Key::Left => GestureKind::SwipeBackward,
                Key::Right => GestureKind::SwipeForward,
                Key::Enter | Key::Space => GestureKind::Tap,
                Key::Down => GestureKind::SwipeDown,
                Key::Up => GestureKind::SwipeUp,
                Key::Escape => return Ok(()),
            };
            match app.input(&InputEvent::gesture(kind)) {
                Some(Action::OpenMenu(id)) => eprintln!("menu: card {id} (placeholder)"),
                Some(Action::RequestSleep) => {
                    eprintln!("sleep: screen off (Enter or an arrow left/right wakes)")
                }
                Some(Action::Back) => eprintln!("back: to the home card"),
                None => {}
            }
        }
        let now = Instant::now();
        app.step(
            (now - last).as_secs_f32(),
            clock((now - start).as_secs_f32()),
        );
        last = now;
        // With gles the frame is read back into the window (slow, fine for development).
        if !app
            .present_if_changed(&mut display)
            .map_err(|e| e.to_string())?
        {
            display.poll();
        }
        std::thread::sleep(frame_time.saturating_sub(now.elapsed()));
    }
    Ok(())
}

#[cfg(not(feature = "window"))]
fn run_window(_: &Args) -> Result<(), String> {
    Err(
        "this build has no window support: rebuild with --features window, or use --headless"
            .into(),
    )
}

fn run_glassd(args: &Args) -> Result<(), String> {
    let path = glassd::socket(args.glassd.clone().flatten());
    let clock = clock_for(args);
    let ctx = clock(0.0);
    let compositor = compositor_for(args.renderer)?;
    let mut app = App::with_compositor(demo::timeline_at(ctx.now_s), ctx, compositor);
    let log = |line: &str| eprintln!("glass-launcher: {line}");
    let mut options = glassd::LoopOptions {
        fps: args.fps.unwrap_or(60),
        ..Default::default()
    };
    if args.window {
        return glassd_window(&path, &mut app, clock, options);
    }
    let mut display = HeadlessDisplay::new(glass_gfx::GLASS_WIDTH, glass_gfx::GLASS_HEIGHT);
    if let Some(out) = &args.out {
        display = display.with_output_dir(out).map_err(|e| e.to_string())?;
    }
    options.max_wait = None;
    glassd::run(
        &path,
        &mut app,
        &mut display,
        || clock(0.0),
        options,
        |_: &mut HeadlessDisplay| true,
        log,
    )
    .map_err(|e| e.to_string())
}

#[cfg(feature = "window")]
fn glassd_window(
    path: &std::path::Path,
    app: &mut App,
    clock: impl Fn(f32) -> RenderContext,
    mut options: glassd::LoopOptions,
) -> Result<(), String> {
    use glass_gfx::window::WindowDisplay;
    let mut display = WindowDisplay::new(
        "glass-launcher (glassd)",
        glass_gfx::GLASS_WIDTH,
        glass_gfx::GLASS_HEIGHT,
    )
    .map_err(|e| e.to_string())?;
    // The window's events must be polled: wait on the socket at most one frame.
    options.max_wait = Some(std::time::Duration::from_millis(16));
    glassd::run(
        path,
        app,
        &mut display,
        || clock(0.0),
        options,
        |d: &mut WindowDisplay| {
            d.poll();
            d.is_open()
        },
        |line| eprintln!("glass-launcher: {line}"),
    )
    .map_err(|e| e.to_string())
}

#[cfg(not(feature = "window"))]
fn glassd_window(
    _: &std::path::Path,
    _: &mut App,
    _: impl Fn(f32) -> RenderContext,
    _: glassd::LoopOptions,
) -> Result<(), String> {
    Err("this build has no window support: rebuild with --features window".into())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            // Bad arguments (or --help): the usage text.
            if !e.is_empty() {
                eprintln!("glass-launcher: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = if args.glassd.is_some() {
        run_glassd(&args)
    } else if args.headless {
        run_headless(&args)
    } else {
        run_window(&args)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // A runtime error (renderer, files, display): the message only.
            eprintln!("glass-launcher: {e}");
            ExitCode::FAILURE
        }
    }
}
