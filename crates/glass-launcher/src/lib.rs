//! The Glass launcher: shows the card timeline and turns input into timeline motion and
//! actions.
//!
//! [`App`] is independent of where input comes from and where frames go. The input is
//! `glassd` ([`glassd`]: `Gesture`/`Scroll` converted field by field into
//! [`glass_ui::InputEvent`], `ScreenRequest{ACTION_SLEEP}` for
//! [`glass_ui::Action::RequestSleep`], the screen following `ScreenState`), a mock script
//! ([`script`]) or the host keyboard.

pub mod clock;
pub mod demo;
pub mod dictation;
pub mod glassd;
pub mod panel;
pub mod script;
pub mod setup;

pub mod platform;
pub mod renderer;
pub mod settings;

use dictation::{Dictation, DictationOut, NOTE_CARD, NOTE_SOURCE};
use glass_api::proto::{DictationState, ErrorCode, Settings, Transcript};
use glass_gfx::{Color, Compositor, Display, Frame, GfxError, SoftwareCompositor};
use glass_ui::{
    Action, Adjuster, CardId, GestureKind, InputEvent, Menu, Outcome, RenderContext, Timeline,
};
use glass_ui::{Card, CardContent};
use panel::Panel;
use platform::PlatformEvent;
use platform::{MockPlatform, Platform};
use settings::{Choice, ServiceInfo, SettingsView};
use setup::{Companion, Setup, SetupEffect};
use std::path::PathBuf;

/// What is shown over the timeline.
enum Overlay {
    /// The menu of card `.1`.
    Menu(Menu<Choice>, CardId),
    /// The wake-angle sub-screen.
    Adjust(Adjuster),
    /// Taking a note by dictation.
    Dictation(Dictation, Panel),
}

/// Which overlay is open (for tests and logs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// A menu, showing item `index`.
    Menu {
        index: usize,
    },
    Adjust,
    Dictation,
}

/// A request for `glassd`, made by the app (the caller sends it).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    SetSettings(Settings),
    /// `DictationStart{request_id, purpose: "free"}`.
    DictationStart(u32),
    DictationCancel(u32),
}

/// The first note card's id; later notes count up.
const FIRST_NOTE_ID: CardId = 1_000;

/// Who decides whether the screen is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenOwner {
    /// A local stand-in for `glassd`'s screen policy (mock scripts, the host window).
    Local,
    /// `glassd`: the app follows `ScreenState` and only asks for sleep.
    Glassd,
}

/// The launcher state: the timeline, the compositor it renders through, and the screen
/// state (owned by `glassd`, or simulated locally without it).
pub struct App {
    timeline: Timeline,
    compositor: Box<dyn Compositor>,
    owner: ScreenOwner,
    screen_on: bool,
    shown: bool,
    overlay: Option<Overlay>,
    platform: Box<dyn Platform>,
    /// The settings glassd applied (locally applied without glassd).
    settings: Option<Settings>,
    service: Option<ServiceInfo>,
    requests: Vec<Request>,
    log: Vec<String>,
    ctx: RenderContext,
    /// The setup flow while it runs, with its card.
    setup: Option<(Setup, Panel)>,
    /// A file whose presence means the setup flow was done.
    setup_marker: Option<PathBuf>,
    companion: Companion,
    next_request: u32,
    next_note: CardId,
}

impl App {
    /// An app rendering in software (tests, debugging).
    pub fn new(timeline: Timeline, ctx: RenderContext) -> App {
        App::with_compositor(timeline, ctx, Box::new(SoftwareCompositor::glass()))
    }

    /// An app rendering through `compositor` (of the Glass display size).
    pub fn with_compositor(
        mut timeline: Timeline,
        ctx: RenderContext,
        compositor: Box<dyn Compositor>,
    ) -> App {
        timeline.set_context(ctx);
        let mut app = App {
            timeline,
            compositor,
            owner: ScreenOwner::Local,
            screen_on: true,
            shown: false,
            overlay: None,
            platform: Box::new(MockPlatform::default()),
            settings: Some(settings::settings_defaults()),
            service: None,
            requests: Vec::new(),
            log: Vec::new(),
            ctx,
            setup: None,
            setup_marker: None,
            companion: Companion::default(),
            next_request: 1,
            next_note: FIRST_NOTE_ID,
        };
        app.refresh_settings_cards();
        app
    }

    /// Starts the setup flow, unless `marker` exists (it is created when the flow ends).
    /// Returns whether the flow runs.
    pub fn start_setup(&mut self, marker: Option<PathBuf>) -> bool {
        if marker.as_ref().is_some_and(|m| m.exists()) {
            return false;
        }
        self.setup_marker = marker;
        let mut s = Setup::new();
        s.set_bluetooth_name(self.platform.bluetooth_name());
        s.set_companion(self.companion.clone());
        let panel = Panel::new(s.card(), self.ctx);
        self.close_overlay();
        self.timeline.release_layers(self.compositor.as_mut());
        self.setup = Some((s, panel));
        self.shown = false;
        true
    }

    /// The setup flow's state, while it runs.
    pub fn setup(&self) -> Option<&Setup> {
        self.setup.as_ref().map(|(s, _)| s)
    }

    fn apply_setup(&mut self, fx: Vec<SetupEffect>) {
        for e in fx {
            match e {
                SetupEffect::Platform(r) => {
                    use platform::PlatformRequest as R;
                    let res = match &r {
                        R::Wifi(on) => self.platform.set_wifi(*on),
                        R::Bluetooth(on) => self.platform.set_bluetooth(*on),
                        R::Discoverable(secs) => self.platform.set_discoverable(*secs),
                        R::AnswerPairing(a) => self.platform.answer_pairing(*a),
                        R::SetProvisioned => self.platform.set_provisioned(),
                        R::OpenApp(p) => self.platform.open_app(p),
                    };
                    if let Err(e) = res {
                        self.log.push(format!("setup: {r:?}: {e}"));
                    }
                }
                SetupEffect::Done { skipped } => {
                    self.log.push(format!(
                        "setup: done{}",
                        if skipped { " (skipped)" } else { "" }
                    ));
                    if let Some(m) = &self.setup_marker {
                        if let Err(e) = std::fs::write(m, b"done\n") {
                            self.log.push(format!("setup: {}: {e}", m.display()));
                        }
                    }
                    if let Some((_, mut panel)) = self.setup.take() {
                        panel.release(self.compositor.as_mut());
                    }
                    self.timeline.show_home();
                    self.shown = false;
                }
            }
        }
        self.refresh_setup_card();
        self.refresh_settings_cards();
    }

    fn refresh_setup_card(&mut self) {
        if let Some((s, panel)) = self.setup.as_mut() {
            panel.set(s.card());
        }
    }

    /// A state change from the platform back end (radios, pairing, set-up state).
    pub fn platform_event(&mut self, e: &PlatformEvent) {
        self.platform.on_event(e);
        if let Some((s, _)) = self.setup.as_mut() {
            let fx = s.platform_event(e);
            self.apply_setup(fx);
        } else {
            self.refresh_settings_cards();
        }
    }

    /// glassd's `CompanionState`: whether a phone is connected through the companion.
    pub fn companion_changed(&mut self, c: Companion) {
        self.companion = c.clone();
        if let Some((s, _)) = self.setup.as_mut() {
            s.set_companion(c);
        }
        self.refresh_setup_card();
        self.refresh_note_card();
    }

    pub fn companion(&self) -> &Companion {
        &self.companion
    }

    fn dictation_offered(&self) -> bool {
        self.owner == ScreenOwner::Glassd
            && self
                .service
                .as_ref()
                .is_some_and(|s| s.capabilities.iter().any(|c| c == "dictation"))
    }

    /// The "Take a note" card: present while glassd relays dictation.
    fn refresh_note_card(&mut self) {
        let exists = self.timeline.cards().iter().any(|c| c.id == NOTE_CARD);
        if !self.dictation_offered() {
            if exists {
                self.timeline.remove_card(NOTE_CARD);
            }
            return;
        }
        let content = CardContent::Settings {
            title: "Take a note".into(),
            subtitle: Some(if self.companion.connected {
                "Tap to speak".into()
            } else {
                "Phone not connected".into()
            }),
        };
        if exists {
            self.timeline.update_card(NOTE_CARD, content);
        } else {
            self.timeline.insert_backward(Card::new(NOTE_CARD, content));
        }
    }

    fn start_dictation(&mut self) {
        let id = self.next_request;
        self.next_request = self.next_request.wrapping_add(1).max(1);
        self.requests.push(Request::DictationStart(id));
        let d = Dictation::new(id);
        let panel = Panel::new(d.card(self.ctx.now_s), self.ctx);
        self.show_overlay(Overlay::Dictation(d, panel));
    }

    fn with_dictation(&mut self, f: impl FnOnce(&mut Dictation) -> Option<DictationOut>) {
        let now = self.ctx.now_s;
        let out = match self.overlay.as_mut() {
            Some(Overlay::Dictation(d, panel)) => {
                let out = f(d);
                panel.set(d.card(now));
                out
            }
            _ => return,
        };
        match out {
            None => {}
            Some(DictationOut::Cancel(id)) => {
                self.requests.push(Request::DictationCancel(id));
                if let Some(Overlay::Dictation(d, _)) = self.overlay.as_mut() {
                    // Already cancelled: closing must not cancel again.
                    d.refused(ErrorCode::Unspecified);
                }
                self.close_overlay();
            }
            Some(DictationOut::Close) => self.close_overlay(),
            Some(DictationOut::Note(text)) => {
                self.close_overlay();
                let id = self.next_note;
                self.next_note += 1;
                self.timeline.insert_history(Card::new(
                    id,
                    CardContent::Text {
                        body: text,
                        source: NOTE_SOURCE.into(),
                        timestamp_s: self.ctx.now_s,
                    },
                ));
                self.log.push(format!("note: card {id}"));
            }
        }
    }

    /// glassd's `DictationState` for a request of this app.
    pub fn dictation_state(&mut self, s: &DictationState) {
        self.with_dictation(|d| d.on_state(s));
    }

    /// glassd's `Transcript` for a request of this app.
    pub fn dictation_transcript(&mut self, t: &Transcript) {
        self.with_dictation(|d| {
            d.on_transcript(t);
            None
        });
    }

    /// glassd refused the `DictationStart` of `request_id`.
    pub fn dictation_refused(&mut self, request_id: u32, code: ErrorCode) {
        self.with_dictation(|d| {
            if d.request_id() == request_id {
                d.refused(code);
            }
            None
        });
    }

    /// Uses `platform` for Wi-Fi and Bluetooth.
    pub fn with_platform(mut self, platform: Box<dyn Platform>) -> App {
        self.platform = platform;
        self.refresh_settings_cards();
        self
    }

    /// What the settings cards show.
    pub fn settings_view(&self) -> SettingsView {
        SettingsView {
            settings: self.settings,
            wifi: self.platform.wifi(),
            bluetooth: self.platform.bluetooth(),
            service: self.service.clone(),
            platform: self.platform.name(),
            renderer: self.compositor.name(),
        }
    }

    /// The applied settings (`None` while waiting for glassd).
    pub fn settings(&self) -> Option<&Settings> {
        self.settings.as_ref()
    }

    /// glassd sent the applied settings (a snapshot, an answer, or a change by someone).
    pub fn settings_changed(&mut self, s: Settings) {
        self.settings = Some(s);
        self.refresh_settings_cards();
    }

    /// The menus offer what is true now (also after a radio or settings change).
    fn refresh_settings_cards(&mut self) {
        self.refresh_settings_card_contents();
        self.refresh_open_menu();
    }

    /// glassd's version and capabilities, for the device card.
    pub fn set_service(&mut self, info: Option<ServiceInfo>) {
        self.service = info;
        self.refresh_settings_cards();
        self.refresh_note_card();
    }

    /// Requests for glassd made since the last call.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// What happened (menu choices, platform errors) since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// The overlay shown, if any.
    pub fn overlay(&self) -> Option<OverlayKind> {
        match &self.overlay {
            None => None,
            Some(Overlay::Menu(m, _)) => Some(OverlayKind::Menu { index: m.index() }),
            Some(Overlay::Adjust(_)) => Some(OverlayKind::Adjust),
            Some(Overlay::Dictation(..)) => Some(OverlayKind::Dictation),
        }
    }

    /// The labels of the open menu's items.
    pub fn menu_labels(&self) -> Vec<String> {
        match &self.overlay {
            Some(Overlay::Menu(m, _)) => m.items().iter().map(|i| i.label.clone()).collect(),
            _ => Vec::new(),
        }
    }

    fn refresh_settings_card_contents(&mut self) {
        let v = self.settings_view();
        for id in settings::ORDER {
            if let Some(c) = settings::card_content(id, &v) {
                let same = self
                    .timeline
                    .cards()
                    .iter()
                    .find(|card| card.id == id)
                    .map(|card| glass_ui::card::render_key(card, &self.ctx))
                    == Some(glass_ui::card::render_key(
                        &glass_ui::Card::new(id, c.clone()),
                        &self.ctx,
                    ));
                if !same {
                    self.timeline.update_card(id, c);
                }
            }
        }
    }

    fn open_menu(&mut self, id: CardId) -> bool {
        let Some(card) = self.timeline.cards().iter().find(|c| c.id == id) else {
            return false;
        };
        let Some((items, first)) = settings::menu_for(card, &self.settings_view()) else {
            return false;
        };
        let Some(mut menu) = Menu::new(items) else {
            return false;
        };
        menu.select_index(first);
        self.show_overlay(Overlay::Menu(menu, id));
        true
    }

    /// After a change of what menus offer (settings, radios): an open menu of a settings
    /// card is rebuilt on the same item, or closed if the card has no menu any more.
    fn refresh_open_menu(&mut self) {
        let Some(Overlay::Menu(m, id)) = &self.overlay else {
            return;
        };
        let (id, index) = (*id, m.index());
        if !settings::ORDER.contains(&id) {
            return;
        }
        let rebuilt = self
            .timeline
            .cards()
            .iter()
            .find(|c| c.id == id)
            .and_then(|card| settings::menu_for(card, &self.settings_view()))
            .and_then(|(items, _)| Menu::new(items));
        match rebuilt {
            Some(mut menu) => {
                menu.select_index(index);
                if let Some(Overlay::Menu(old, _)) = self.overlay.as_mut() {
                    old.release(self.compositor.as_mut());
                }
                self.overlay = Some(Overlay::Menu(menu, id));
                self.shown = false;
            }
            None => self.close_overlay(),
        }
    }

    fn show_overlay(&mut self, o: Overlay) {
        self.close_overlay();
        // The overlay covers the timeline: free its layers (GPU memory budget).
        self.timeline.release_layers(self.compositor.as_mut());
        self.overlay = Some(o);
        self.shown = false;
    }

    fn close_overlay(&mut self) {
        match self.overlay.take() {
            Some(Overlay::Menu(mut m, _)) => m.release(self.compositor.as_mut()),
            Some(Overlay::Adjust(mut a)) => a.release(self.compositor.as_mut()),
            Some(Overlay::Dictation(d, mut panel)) => {
                if d.is_open() {
                    self.requests.push(Request::DictationCancel(d.request_id()));
                }
                panel.release(self.compositor.as_mut());
            }
            None => return,
        }
        self.shown = false;
    }

    fn request_settings(&mut self, patch: Settings) {
        match self.owner {
            ScreenOwner::Glassd => self.requests.push(Request::SetSettings(patch)),
            ScreenOwner::Local => {
                let current = self
                    .settings
                    .get_or_insert_with(settings::settings_defaults);
                settings::apply_locally(current, &patch);
                self.refresh_settings_cards();
            }
        }
    }

    /// Asks for a settings change (with glassd: queued as a [`Request`]; without:
    /// applied locally with glassd's ranges).
    pub fn change_settings(&mut self, patch: Settings) {
        self.request_settings(patch);
    }

    fn apply(&mut self, choice: Choice) {
        match choice {
            Choice::Patch(p) => {
                self.log.push(format!("settings: {p:?}"));
                self.request_settings(p);
            }
            Choice::Wifi(on) => {
                if let Err(e) = self.platform.set_wifi(on) {
                    self.log.push(format!("wi-fi: {e}"));
                }
                self.refresh_settings_cards();
            }
            Choice::Bluetooth(on) => {
                if let Err(e) = self.platform.set_bluetooth(on) {
                    self.log.push(format!("bluetooth: {e}"));
                }
                self.refresh_settings_cards();
            }
            Choice::WakeAngle => {
                let a = settings::wake_angle_adjuster(&self.settings_view());
                self.show_overlay(Overlay::Adjust(a));
            }
            Choice::Delete(id) => {
                self.log.push(format!("delete: card {id}"));
                self.timeline.remove_card(id);
            }
        }
    }

    fn overlay_input(&mut self, event: &InputEvent) {
        let choice = match self.overlay.as_mut() {
            Some(Overlay::Menu(m, _)) => match m.handle(event) {
                None => return,
                Some(Outcome::Closed) => None,
                Some(Outcome::Selected(c)) => Some(c),
            },
            Some(Overlay::Dictation(..)) => {
                self.with_dictation(|d| d.input(event));
                return;
            }
            Some(Overlay::Adjust(a)) => match a.handle(event) {
                None => return,
                Some(Outcome::Closed) => None,
                Some(Outcome::Selected(v)) => Some(Choice::Patch(Settings {
                    head_wake_angle_deg: Some(v),
                    ..Default::default()
                })),
            },
            None => return,
        };
        self.close_overlay();
        if let Some(c) = choice {
            self.apply(c);
        }
    }

    /// The renderer's name ("software", "gles").
    pub fn renderer(&self) -> &'static str {
        self.compositor.name()
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Whether the screen is on (or dimmed).
    pub fn screen_on(&self) -> bool {
        self.screen_on
    }

    /// Lets `glassd` own the screen: the local wake stand-in is off, and the screen changes
    /// only through [`App::screen_off`] and [`App::screen_wake`].
    pub fn follow_glassd(&mut self) {
        if self.owner != ScreenOwner::Glassd {
            self.owner = ScreenOwner::Glassd;
            // Shown once glassd sends them.
            self.settings = None;
            self.refresh_settings_cards();
        }
    }

    pub fn screen_owner(&self) -> ScreenOwner {
        self.owner
    }

    /// The screen went off: motion stops where it was heading, and nothing is drawn.
    pub fn screen_off(&mut self) {
        self.close_overlay();
        self.timeline.stop_motion();
        self.screen_on = false;
        self.shown = false;
    }

    /// The screen is on again; `return_home` shows the home card at once.
    pub fn screen_wake(&mut self, return_home: bool) {
        if return_home {
            self.timeline.show_home();
        }
        self.screen_on = true;
        self.shown = false;
    }

    /// Ends a touch whose end will not arrive (focus lost, glassd gone); see
    /// [`glass_ui::Timeline::cancel_touch`].
    pub fn cancel_touch(&mut self) {
        self.timeline.cancel_touch();
        if let Some(Overlay::Menu(m, _)) = self.overlay.as_mut() {
            m.cancel_touch();
        }
    }

    /// The connection to glassd is gone: its settings are unknown again and requests made
    /// for it are dropped (a new session sends a fresh snapshot).
    pub fn glassd_disconnected(&mut self) {
        self.requests.clear();
        self.service = None;
        if self.owner == ScreenOwner::Glassd {
            self.settings = None;
        }
        self.refresh_settings_cards();
    }

    /// Whether the timeline is moving (frames are due at the frame rate).
    pub fn is_animating(&self) -> bool {
        let menu_moving = matches!(&self.overlay, Some(Overlay::Menu(m, _)) if !m.is_settled());
        menu_moving || !self.timeline.is_settled()
    }

    /// Handles one input event.
    ///
    /// While the (simulated) screen is off, this stands in for `glassd`'s screen policy: a
    /// tap or a forward or backward swipe wakes the screen and is consumed; `Scroll` frames
    /// and other gestures are ignored. With `glassd`, none of these events reach the
    /// launcher while the screen is off, and the dim step (where the restoring touch is
    /// consumed too) is handled there; the launcher only follows `ScreenState`.
    pub fn input(&mut self, event: &InputEvent) -> Option<Action> {
        if self.owner == ScreenOwner::Glassd {
            // glassd delivers no touchpad events while the screen is off, and changes the
            // screen itself (the caller sends ScreenRequest for RequestSleep).
            if !self.screen_on {
                return None;
            }
            return self.timeline_input(event);
        }
        if !self.screen_on {
            if let InputEvent::Gesture {
                kind: GestureKind::Tap | GestureKind::SwipeForward | GestureKind::SwipeBackward,
                ..
            } = event
            {
                self.screen_on = true;
                self.shown = false;
            }
            return None;
        }
        let action = self.timeline_input(event);
        if action == Some(Action::RequestSleep) {
            // With glassd: send ScreenRequest{ACTION_SLEEP} and follow ScreenState.
            self.screen_on = false;
            self.shown = false;
        }
        action
    }

    /// Input with the screen on: the overlay if one is open, else the timeline (whose
    /// `TAP` opens the card's menu).
    fn timeline_input(&mut self, event: &InputEvent) -> Option<Action> {
        if let Some((s, _)) = self.setup.as_mut() {
            let fx = s.input(event);
            self.apply_setup(fx);
            return None;
        }
        if self.overlay.is_some() {
            self.overlay_input(event);
            return None;
        }
        let action = self.timeline.handle(event);
        if action == Some(Action::OpenMenu(NOTE_CARD)) && self.dictation_offered() {
            self.start_dictation();
            return action;
        }
        if let Some(Action::OpenMenu(id)) = action {
            if !self.open_menu(id) {
                return None; // this card has no menu
            }
        }
        action
    }

    /// Advances time by `dt` seconds with the wall clock `ctx`.
    pub fn step(&mut self, dt: f32, ctx: RenderContext) {
        self.ctx = ctx;
        self.timeline.set_context(ctx);
        self.timeline.tick(dt);
        match &mut self.overlay {
            Some(Overlay::Menu(m, _)) => m.tick(dt),
            Some(Overlay::Adjust(a)) => a.set_context(ctx),
            Some(Overlay::Dictation(_, p)) => p.set_context(ctx),
            None => {}
        }
        if let Some((_, p)) = self.setup.as_mut() {
            p.set_context(ctx);
        }
    }

    /// Composes a new frame if anything changed since the last one; true if it did.
    pub fn render_if_changed(&mut self) -> Result<bool, GfxError> {
        if !self.screen_on {
            // With glassd the panel is off: nothing to draw until the wake.
            if self.shown || self.owner == ScreenOwner::Glassd {
                return Ok(false);
            }
            self.compositor.compose(Color::BLACK, &[])?;
            self.shown = true;
            return Ok(true);
        }
        if let Some((_, p)) = self.setup.as_mut() {
            if self.shown && !p.needs_redraw() {
                return Ok(false);
            }
            p.render(self.compositor.as_mut())?;
            self.shown = true;
            return Ok(true);
        }
        match &mut self.overlay {
            Some(Overlay::Dictation(_, p)) => {
                if self.shown && !p.needs_redraw() {
                    return Ok(false);
                }
                p.render(self.compositor.as_mut())?;
            }
            Some(Overlay::Menu(m, _)) => {
                if self.shown && !m.needs_redraw() {
                    return Ok(false);
                }
                m.render(self.compositor.as_mut())?;
            }
            Some(Overlay::Adjust(a)) => {
                if self.shown && !a.needs_redraw() {
                    return Ok(false);
                }
                a.render(self.compositor.as_mut())?;
            }
            None => {
                if self.shown && !self.timeline.needs_redraw() {
                    return Ok(false);
                }
                self.timeline.render(self.compositor.as_mut())?;
            }
        }
        self.shown = true;
        Ok(true)
    }

    /// Composes and presents a new frame if anything changed; true if one was presented.
    pub fn present_if_changed(&mut self, display: &mut dyn Display) -> Result<bool, GfxError> {
        if !self.render_if_changed()? {
            return Ok(false);
        }
        self.compositor.present(display)?;
        Ok(true)
    }

    /// How many layers the compositor holds (tests: the GPU memory bound).
    pub fn layer_count(&self) -> usize {
        self.compositor.layer_count()
    }

    /// The last composed frame, read back (tests).
    pub fn read_frame(&mut self) -> Result<Frame, GfxError> {
        self.compositor.read_frame()
    }
}
