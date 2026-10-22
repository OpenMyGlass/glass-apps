//! Mock input scripts, in the command syntax of `glassd --mock` (see the glass-services
//! interface): one command per line, `#` starts a comment.
//!
//! | Command | Effect here |
//! |---|---|
//! | `tap [fingers]`, `longpress` | the gesture |
//! | `swipe forward\|backward\|down\|up [fingers]` | the gesture alone, with no `Scroll` before it |
//! | `scroll <displacement>` | a slow drag: `Scroll` frames over [`SCROLL_FRAMES`] frames up to `displacement` (pad widths, + = forward), then `Scroll{ended}` at rest, so the timeline settles on the nearest card |
//! | `wait <ms>`, `wait <n>ms`, `wait <n>s` | time passes; frames are presented only when something changed |
//! | `camera`, `lookup`, `lookdown`, `wear`, `wink` | accepted and ignored (no effect on the timeline) |

use crate::App;
use glass_gfx::{Display, GfxError};
use glass_ui::{Action, GestureKind, InputEvent, RenderContext};

/// Frames a `scroll` command takes to reach its displacement.
pub const SCROLL_FRAMES: u32 = 6;

/// How long the runner lets an animation finish after the last command.
const SETTLE_LIMIT_S: f32 = 2.0;

/// One script command.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// A gesture with a finger count.
    Gesture(GestureKind, u32),
    /// A drag to this displacement, then release at rest.
    Scroll(f32),
    /// Milliseconds of time passing.
    Wait(u32),
    /// A mock command with no effect on the launcher.
    Ignored(String),
}

/// Parses a script. Errors name the line.
pub fn parse(script: &str) -> Result<Vec<Command>, String> {
    let mut out = Vec::new();
    for (n, raw) in script.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let cmd = parse_line(line).map_err(|e| format!("line {}: {e}: {line:?}", n + 1))?;
        out.push(cmd);
    }
    Ok(out)
}

fn fingers(arg: Option<&str>) -> Result<u32, String> {
    match arg {
        None => Ok(1),
        Some(s) => match s.parse::<u32>() {
            Ok(n @ 1..=3) => Ok(n),
            _ => Err("fingers must be 1, 2 or 3".into()),
        },
    }
}

fn parse_line(line: &str) -> Result<Command, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let arg = |i: usize| words.get(i).copied();
    let too_many = |n: usize| -> Result<(), String> {
        if words.len() > n {
            Err("too many arguments".into())
        } else {
            Ok(())
        }
    };
    match words[0] {
        "tap" => {
            too_many(2)?;
            Ok(Command::Gesture(GestureKind::Tap, fingers(arg(1))?))
        }
        "longpress" => {
            too_many(1)?;
            Ok(Command::Gesture(GestureKind::LongPress, 1))
        }
        "swipe" => {
            too_many(3)?;
            let kind = match arg(1) {
                Some("forward") => GestureKind::SwipeForward,
                Some("backward") => GestureKind::SwipeBackward,
                Some("down") => GestureKind::SwipeDown,
                Some("up") => GestureKind::SwipeUp,
                _ => return Err("swipe needs forward, backward, down or up".into()),
            };
            Ok(Command::Gesture(kind, fingers(arg(2))?))
        }
        "scroll" => {
            too_many(2)?;
            let d = arg(1)
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|d| d.is_finite())
                .ok_or("scroll needs a displacement in pad widths")?;
            Ok(Command::Scroll(d))
        }
        "wait" => {
            too_many(2)?;
            let s = arg(1).ok_or("wait needs a duration")?;
            let ms = if let Some(v) = s.strip_suffix("ms") {
                v.parse::<u32>().ok()
            } else if let Some(v) = s.strip_suffix('s') {
                v.parse::<u32>().ok().and_then(|v| v.checked_mul(1000))
            } else {
                s.parse::<u32>().ok()
            };
            Ok(Command::Wait(ms.ok_or("bad duration")?))
        }
        "camera" | "lookup" | "lookdown" | "wear" | "wink" => Ok(Command::Ignored(line.into())),
        _ => Err("unknown command".into()),
    }
}

/// Runs scripts against an [`App`] at a fixed frame rate and presents changed frames.
pub struct Runner {
    fps: u32,
    log: Vec<String>,
    elapsed_s: f32,
}

impl Runner {
    pub fn new(fps: u32) -> Runner {
        Runner {
            fps: fps.max(1),
            log: Vec::new(),
            elapsed_s: 0.0,
        }
    }

    /// What happened, one line per action (menus, sleep, back).
    pub fn log(&self) -> &[String] {
        &self.log
    }

    fn dt(&self) -> f32 {
        1.0 / self.fps as f32
    }

    /// Runs `commands`, then lets motion settle. `clock` gives the wall clock for a time in
    /// seconds since the start (a fixed clock makes the output reproducible).
    pub fn run(
        &mut self,
        commands: &[Command],
        app: &mut App,
        display: &mut dyn Display,
        clock: impl Fn(f32) -> RenderContext,
    ) -> Result<(), GfxError> {
        self.present(app, display)?;
        for cmd in commands {
            match cmd {
                Command::Gesture(kind, fingers) => {
                    self.input(
                        app,
                        InputEvent::Gesture {
                            kind: *kind,
                            fingers: *fingers,
                            velocity: 0.0,
                            timestamp_us: self.timestamp_us(),
                        },
                    );
                    self.frame(app, display, &clock)?;
                }
                Command::Scroll(d) => {
                    let dt = self.dt();
                    let mut prev = 0.0;
                    for i in 1..=SCROLL_FRAMES {
                        let disp = d * i as f32 / SCROLL_FRAMES as f32;
                        self.input(
                            app,
                            InputEvent::Scroll {
                                displacement: disp,
                                delta: disp - prev,
                                velocity: (disp - prev) / dt,
                                fingers: 1,
                                ended: false,
                                cancelled: false,
                                timestamp_us: self.timestamp_us(),
                            },
                        );
                        prev = disp;
                        self.frame(app, display, &clock)?;
                    }
                    self.input(
                        app,
                        InputEvent::Scroll {
                            displacement: *d,
                            delta: 0.0,
                            velocity: 0.0,
                            fingers: 1,
                            ended: true,
                            cancelled: false,
                            timestamp_us: self.timestamp_us(),
                        },
                    );
                    self.frame(app, display, &clock)?;
                }
                Command::Wait(ms) => {
                    let frames = (*ms as u64 * self.fps as u64).div_ceil(1000);
                    for _ in 0..frames {
                        self.frame(app, display, &clock)?;
                    }
                }
                Command::Ignored(line) => self.log.push(format!("ignored: {line}")),
            }
        }
        let limit = (SETTLE_LIMIT_S * self.fps as f32) as u32;
        for _ in 0..limit {
            if app.timeline().is_settled() {
                break;
            }
            self.frame(app, display, &clock)?;
        }
        self.present(app, display)
    }

    fn timestamp_us(&self) -> u64 {
        (self.elapsed_s as f64 * 1e6) as u64
    }

    fn input(&mut self, app: &mut App, event: InputEvent) {
        match app.input(&event) {
            Some(Action::OpenMenu(id)) => self.log.push(format!("menu: card {id}")),
            Some(Action::RequestSleep) => self.log.push("sleep: screen off".into()),
            Some(Action::Back) => self.log.push("back: to the home card".into()),
            None => {}
        }
        self.log.extend(app.take_log());
    }

    /// One frame of time: advance, then present if changed.
    fn frame(
        &mut self,
        app: &mut App,
        display: &mut dyn Display,
        clock: &impl Fn(f32) -> RenderContext,
    ) -> Result<(), GfxError> {
        let dt = self.dt();
        self.elapsed_s += dt;
        app.step(dt, clock(self.elapsed_s));
        self.present(app, display)
    }

    fn present(&mut self, app: &mut App, display: &mut dyn Display) -> Result<(), GfxError> {
        app.present_if_changed(display)?;
        Ok(())
    }
}
