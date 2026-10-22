//! The launcher as a `glassd` client (glass-services `docs/api.md`, version 1).
//!
//! - **Session:** `Hello` as `glass-launcher`, `Subscribe` to `TOPIC_GESTURE`,
//!   `TOPIC_SCREEN`, `TOPIC_SETTINGS` and `TOPIC_WEAR` (wear is not used yet), then
//!   `AcquireInputFocus`.
//! - **Input:** `Gesture` and `Scroll` become [`InputEvent`]s field by field
//!   ([`to_input`]); they are ignored while the launcher does not hold the focus
//!   (`FocusState{focused:false}`, an app is in front).
//! - **Screen:** `glassd` owns it. [`Action::RequestSleep`] sends
//!   `ScreenRequest{ACTION_SLEEP}`; the app follows `ScreenState`: nothing is drawn while
//!   `SCREEN_OFF`; on a wake the home card is shown unless [`returns_home`] says otherwise.
//! - **Loop:** [`run`] blocks on the socket: until the next frame while the timeline moves,
//!   until the next minute (the home clock) at rest, without a timeout while the screen is
//!   off. When `glassd` goes away it reconnects with a [`Backoff`].

use crate::settings::ServiceInfo;
use crate::{App, Request};
use glass_api::proto::{
    client_frame, screen_request, service_frame, ErrorCode, GestureKind as ProtoKind, Screen,
    ScreenRequest, ScreenState, ServiceFrame, SetSettings, Settings, Topic,
};
use glass_api::{Client, ClientError};
use glass_gfx::{Display, GfxError};
use glass_ui::{Action, GestureKind, InputEvent, RenderContext};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The name in `Hello`.
pub const CLIENT_NAME: &str = "glass-launcher";

/// The subscribed topics.
pub const TOPICS: [Topic; 4] = [Topic::Gesture, Topic::Screen, Topic::Settings, Topic::Wear];

/// A wake within this time of a timeout or gesture sleep keeps the card that was shown.
pub const STAY_ON_CARD_MS: u32 = 5_000;

/// The touchpad event in a service frame, converted 1:1; `None` for other frames and for
/// gesture kinds this build does not know.
pub fn to_input(body: &service_frame::Body) -> Option<InputEvent> {
    match body {
        service_frame::Body::Gesture(g) => {
            let kind = match ProtoKind::try_from(g.kind).ok()? {
                ProtoKind::Tap => GestureKind::Tap,
                ProtoKind::LongPress => GestureKind::LongPress,
                ProtoKind::SwipeForward => GestureKind::SwipeForward,
                ProtoKind::SwipeBackward => GestureKind::SwipeBackward,
                ProtoKind::SwipeDown => GestureKind::SwipeDown,
                ProtoKind::SwipeUp => GestureKind::SwipeUp,
                ProtoKind::Unspecified => return None,
            };
            Some(InputEvent::Gesture {
                kind,
                fingers: g.fingers,
                velocity: g.velocity,
                timestamp_us: g.timestamp_us,
            })
        }
        service_frame::Body::Scroll(s) => Some(InputEvent::Scroll {
            displacement: s.displacement,
            delta: s.delta,
            velocity: s.velocity,
            fingers: s.fingers,
            ended: s.ended,
            cancelled: s.cancelled,
            timestamp_us: s.timestamp_us,
        }),
        _ => None,
    }
}

/// Whether a wake shows the home card (docs/api.md, Screen): yes, unless the last sleep
/// was a timeout or a gesture (`nudge`, `two-finger-swipe-down`) and the screen was off for
/// [`STAY_ON_CARD_MS`] or less. A swipe wake always returns home. (Stock Glass also stays
/// on the card when an immersive app was in front; there are no apps yet.)
pub fn returns_home(wake: &ScreenState) -> bool {
    if wake.reason == "swipe-forward" || wake.reason == "swipe-backward" {
        return true;
    }
    let soft_sleep = matches!(
        wake.last_sleep_reason.as_str(),
        "timeout" | "nudge" | "two-finger-swipe-down"
    );
    !(soft_sleep && wake.off_ms <= STAY_ON_CARD_MS)
}

/// Reconnection delays: from `min`, doubling up to `max`; reset after a good connection.
#[derive(Clone, Debug)]
pub struct Backoff {
    min: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(min: Duration, max: Duration) -> Backoff {
        Backoff {
            min,
            max,
            next: min,
        }
    }

    /// The delay before the next attempt.
    pub fn next_delay(&mut self) -> Duration {
        let d = self.next;
        self.next = (self.next * 2).min(self.max);
        d
    }

    pub fn reset(&mut self) {
        self.next = self.min;
    }
}

impl Default for Backoff {
    /// 100 ms doubling up to 5 s.
    fn default() -> Backoff {
        Backoff::new(Duration::from_millis(100), Duration::from_secs(5))
    }
}

/// An error of the glassd mode.
#[derive(Debug)]
pub enum GlassdError {
    Client(ClientError),
    Render(GfxError),
}

impl std::fmt::Display for GlassdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GlassdError::Client(e) => write!(f, "glassd: {e}"),
            GlassdError::Render(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for GlassdError {}

impl From<ClientError> for GlassdError {
    fn from(e: ClientError) -> Self {
        GlassdError::Client(e)
    }
}

impl From<GfxError> for GlassdError {
    fn from(e: GfxError) -> Self {
        GlassdError::Render(e)
    }
}

/// One connection to `glassd`.
pub struct Session {
    client: Client,
    focused: bool,
    settings: Option<Settings>,
    log: Vec<String>,
}

impl Session {
    /// Connects, says hello, subscribes and asks for the input focus.
    pub fn connect(path: &Path) -> Result<Session, ClientError> {
        let mut client = Client::connect_as(path, CLIENT_NAME)?;
        if client.welcome().api_version < 1 {
            return Err(ClientError::Handshake(Box::new(ServiceFrame {
                body: Some(service_frame::Body::Welcome(client.welcome().clone())),
            })));
        }
        client.subscribe(&TOPICS)?;
        client.acquire_focus()?;
        Ok(Session {
            client,
            focused: false,
            settings: None,
            log: Vec::new(),
        })
    }

    /// glassd's version and capabilities from its `Welcome`.
    pub fn service_info(&self) -> ServiceInfo {
        let w = self.client.welcome();
        ServiceInfo {
            version: w.service_version.clone(),
            capabilities: w.capabilities.clone(),
        }
    }

    /// Whether glassd says the launcher holds the input focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// The last `Settings` from glassd.
    pub fn settings(&self) -> Option<&Settings> {
        self.settings.as_ref()
    }

    /// What happened (actions, errors from glassd), oldest first; drained.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// The service's capabilities.
    pub fn capabilities(&self) -> &[String] {
        &self.client.welcome().capabilities
    }

    /// Applies one frame from glassd to `app`.
    pub fn handle(&mut self, frame: ServiceFrame, app: &mut App) -> Result<(), ClientError> {
        let Some(body) = frame.body else {
            return Ok(());
        };
        if let Some(event) = to_input(&body) {
            if !self.focused {
                return Ok(());
            }
            match app.input(&event) {
                Some(Action::RequestSleep) => {
                    self.client.send(client_frame::Body::Screen(ScreenRequest {
                        action: screen_request::Action::Sleep as i32,
                        duration_ms: 0,
                    }))?;
                    self.log.push("sleep: ScreenRequest{ACTION_SLEEP}".into());
                }
                Some(Action::OpenMenu(id)) => self.log.push(format!("menu: card {id}")),
                Some(Action::Back) => self.log.push("back: to the home card".into()),
                None => {}
            }
            self.log.extend(app.take_log());
            return self.send_requests(app);
        }
        match body {
            service_frame::Body::FocusState(f) => {
                // glassd sends FocusState{false} before the final Scroll{ended, cancelled},
                // which is then no longer ours: end the touch here.
                if self.focused && !f.focused {
                    app.cancel_touch();
                }
                self.focused = f.focused;
            }
            service_frame::Body::ScreenState(s) => apply_screen(app, &s),
            service_frame::Body::Settings(s) => {
                self.settings = Some(s);
                app.settings_changed(s);
            }
            service_frame::Body::Error(e) => {
                let code = ErrorCode::try_from(e.code)
                    .map(|c| c.as_str_name().to_string())
                    .unwrap_or_else(|_| format!("error code {}", e.code));
                self.log
                    .push(format!("glassd {code}: {} (seq {})", e.message, e.seq));
            }
            // Wear and the rest: subscribed for later milestones.
            _ => {}
        }
        Ok(())
    }

    /// Sends the requests the app made (settings changes chosen in a menu).
    pub fn send_requests(&mut self, app: &mut App) -> Result<(), ClientError> {
        for req in app.take_requests() {
            match req {
                Request::SetSettings(patch) => {
                    self.client
                        .send(client_frame::Body::SetSettings(SetSettings {
                            settings: Some(patch),
                        }))?;
                    self.log.push(format!("SetSettings {patch:?}"));
                }
            }
        }
        Ok(())
    }

    /// Sends pending requests, then waits up to `timeout` (`None`: no limit) for a frame,
    /// applies it and every frame already received; returns how many were applied.
    pub fn pump(&mut self, app: &mut App, timeout: Option<Duration>) -> Result<usize, ClientError> {
        self.send_requests(app)?;
        let mut n = 0;
        let Some(first) = self.client.recv(timeout)? else {
            return Ok(0);
        };
        self.handle(first, app)?;
        n += 1;
        // Frames already decoded or already in the socket: a 1 µs deadline reads what is
        // there and does not wait for more.
        while let Some(f) = self.client.recv(Some(Duration::from_micros(1)))? {
            self.handle(f, app)?;
            n += 1;
        }
        Ok(n)
    }
}

fn apply_screen(app: &mut App, s: &ScreenState) {
    match Screen::try_from(s.screen) {
        Ok(Screen::Off) => app.screen_off(),
        // A wake (or the first snapshot of a lit screen).
        Ok(Screen::On) | Ok(Screen::Dim) if !app.screen_on() => {
            let home = s.reason != "snapshot" && returns_home(s);
            app.screen_wake(home);
        }
        _ => {}
    }
}

/// Settings of [`run`].
pub struct LoopOptions {
    /// Frame rate while the timeline moves.
    pub fps: u32,
    /// Longest wait on the socket (a host window needs its events polled), `None` for none.
    pub max_wait: Option<Duration>,
    pub backoff: Backoff,
}

impl Default for LoopOptions {
    fn default() -> LoopOptions {
        LoopOptions {
            fps: 60,
            max_wait: None,
            backoff: Backoff::default(),
        }
    }
}

/// How long the loop may block: a frame while moving, until the next minute at rest (the
/// home clock), forever while the screen is off; never more than `max_wait`.
pub fn wait_time(app: &App, now_s: i64, fps: u32, max_wait: Option<Duration>) -> Option<Duration> {
    let wait = if !app.screen_on() {
        None
    } else if app.is_animating() {
        Some(Duration::from_micros(1_000_000 / fps.max(1) as u64))
    } else {
        let to_minute = 60 - now_s.rem_euclid(60) as u64;
        Some(Duration::from_secs(to_minute) + Duration::from_millis(5))
    };
    match (wait, max_wait) {
        (Some(w), Some(m)) => Some(w.min(m)),
        (w, m) => w.or(m),
    }
}

/// The socket to use: the one given, else glass-api's (`$GLASS_SERVICES_SOCKET`, else the
/// platform default).
pub fn socket(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(glass_api::socket_path)
}

/// Runs the launcher against glassd at `path` until `keep_going` returns false (checked
/// after every wake-up of the loop, with the display, so a host window can poll its events
/// and stop when closed). Reconnects when glassd goes away. `log` receives what happened.
pub fn run<D: Display>(
    path: &Path,
    app: &mut App,
    display: &mut D,
    clock: impl Fn() -> RenderContext,
    mut options: LoopOptions,
    mut keep_going: impl FnMut(&mut D) -> bool,
    mut log: impl FnMut(&str),
) -> Result<(), GlassdError> {
    app.follow_glassd();
    let mut last = Instant::now();
    let mut step = |app: &mut App, display: &mut D| -> Result<(), GlassdError> {
        let now = Instant::now();
        app.step((now - last).as_secs_f32(), clock());
        last = now;
        app.present_if_changed(display)?;
        Ok(())
    };
    while keep_going(display) {
        let mut session = match Session::connect(path) {
            Ok(s) => s,
            Err(e) => {
                let d = options.backoff.next_delay();
                log(&format!(
                    "glassd at {} unavailable ({e}); retrying in {} ms",
                    path.display(),
                    d.as_millis()
                ));
                // Until the retry: keep the clock current (no input arrives) and, with a
                // window, its events polled every `max_wait`.
                let retry_at = Instant::now() + d;
                loop {
                    step(app, display)?;
                    let left = retry_at.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    std::thread::sleep(options.max_wait.map_or(left, |m| left.min(m)));
                    if !keep_going(display) {
                        return Ok(());
                    }
                }
                continue;
            }
        };
        options.backoff.reset();
        // A touch from before the connection was lost never ends: drop it.
        app.cancel_touch();
        app.set_service(Some(session.service_info()));
        log(&format!(
            "connected to glassd at {} (capabilities: {})",
            path.display(),
            session.capabilities().join(", ")
        ));
        loop {
            step(app, display)?;
            if !keep_going(display) {
                return Ok(());
            }
            let wait = wait_time(app, clock().now_s, options.fps, options.max_wait);
            let pumped = session.pump(app, wait);
            for line in session.take_log() {
                log(&line);
            }
            if let Err(e) = pumped {
                app.glassd_disconnected();
                log(&format!("glassd connection lost ({e}); reconnecting"));
                break;
            }
        }
    }
    Ok(())
}
