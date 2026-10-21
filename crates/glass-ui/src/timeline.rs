//! The card timeline.
//!
//! Cards are kept in one ordered list. The home card is in the middle; the cards on the
//! *backward* side (lower indices, to the left on screen) are "now" cards and, farthest,
//! the settings; the cards on the *forward* side (higher indices, to the right) are the
//! history, newest first. `SWIPE_FORWARD` moves to higher indices (older cards).
//!
//! # Motion
//!
//! The position is a real number in card units (`3.0` = card 3 fills the screen,
//! `3.4` = card 3 moved 40 % of the width to the left with card 4 following it).
//!
//! - **Drag:** while `Scroll` events arrive, the position follows the finger:
//!   `start + displacement × CARDS_PER_PAD_WIDTH`. One full touchpad width
//!   ([`CARDS_PER_PAD_WIDTH`] = 2) moves two cards, so a comfortable third of the pad moves
//!   about two thirds of a card, and the release (or the swipe gesture) completes it. Past
//!   either end the motion is damped ([`EDGE_RESISTANCE`], at most [`MAX_OVERSCROLL`]).
//! - **Release:** the final `Scroll{ended}` settles on `round(position + velocity ×
//!   CARDS_PER_PAD_WIDTH × FLING_PROJECTION_S)`; a release faster than
//!   [`FLING_MIN_VELOCITY`] moves at least one card in its direction. A cancelled touch
//!   returns to the card where the drag started.
//! - **Swipe after a drag:** glass-services sends a horizontal swipe as Scroll frames, the
//!   final `Scroll{ended}`, then the `SWIPE_*` gesture. A swipe within
//!   [`SWIPE_AFTER_SCROLL_S`] of a release is the same touch: it makes sure the settle target
//!   is at least one card away from the drag's start in its direction, and is not counted a
//!   second time. A swipe without a drag moves one card; swipes during an animation add up.
//! - **Animation:** from the current position to the target with an ease-out cubic curve over
//!   `min(0.12 s + 0.12 s × distance, 0.45 s)`. Time only advances through [`Timeline::tick`],
//!   so animation is deterministic for any sequence of time steps and lands exactly on the
//!   target.
//!
//! # Rendering
//!
//! Each card is rendered once into its own layer ([`crate::card::render_card`]); a frame is
//! one or two layer copies at the scroll offset. A layer is re-rendered only when its
//! [`crate::card::render_key`] changes (content, clock minute, relative time text). When the
//! timeline is at rest, the cards next to the visible one are rendered ahead and layers
//! farther away are dropped, so a drag never renders a card and memory stays at about three
//! layers (about 900 KiB each at 640×360).

use crate::card::{render_card, render_key, Card, CardContent, CardId, RenderContext};
use crate::input::{GestureKind, InputEvent};
use glass_gfx::{Color, Frame, GLASS_HEIGHT, GLASS_WIDTH};
use std::collections::HashMap;

/// Cards moved by a drag across the full touchpad width.
pub const CARDS_PER_PAD_WIDTH: f32 = 2.0;
/// How far ahead (seconds) the release velocity is projected to choose where to settle.
pub const FLING_PROJECTION_S: f32 = 0.15;
/// Release velocity (pad widths per second) above which at least one card is moved.
pub const FLING_MIN_VELOCITY: f32 = 0.5;
/// Time after a release within which a swipe gesture belongs to the same touch.
pub const SWIPE_AFTER_SCROLL_S: f32 = 0.1;
/// Fraction of the drag applied past either end of the timeline.
pub const EDGE_RESISTANCE: f32 = 0.3;
/// Largest overscroll past either end, in cards.
pub const MAX_OVERSCROLL: f32 = 0.3;

/// What the launcher should do after an input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `SWIPE_DOWN` on the home card: turn the screen off (`ScreenRequest{ACTION_SLEEP}`).
    RequestSleep,
    /// `TAP`: open the menu of this card.
    OpenMenu(CardId),
    /// `SWIPE_DOWN` elsewhere: the timeline is going back to the home card.
    Back,
}

#[derive(Clone, Copy, Debug)]
struct Animation {
    from: f32,
    to: f32,
    elapsed: f32,
    duration: f32,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    start_pos: f32,
    start_index: usize,
}

#[derive(Clone, Copy, Debug)]
struct Release {
    start_index: usize,
    age: f32,
}

struct Layer {
    key: u64,
    frame: Frame,
}

/// The card timeline: model, input handling, motion and rendering.
pub struct Timeline {
    cards: Vec<Card>,
    home_id: CardId,
    width: u32,
    height: u32,
    pos: f32,
    target: usize,
    anim: Option<Animation>,
    drag: Option<Drag>,
    release: Option<Release>,
    ctx: RenderContext,
    layers: HashMap<CardId, Layer>,
    renders: u64,
    dirty: bool,
}

impl Timeline {
    /// A timeline at the Glass display size, on the home card.
    ///
    /// `backward` is ordered from the card next to home to the farthest (settings last);
    /// `history` from the newest to the oldest.
    pub fn new(backward: Vec<Card>, home: Card, history: Vec<Card>) -> Timeline {
        let home_index = backward.len();
        let home_id = home.id;
        let mut cards: Vec<Card> = backward.into_iter().rev().collect();
        cards.push(home);
        cards.extend(history);
        Timeline {
            cards,
            home_id,
            width: GLASS_WIDTH,
            height: GLASS_HEIGHT,
            pos: home_index as f32,
            target: home_index,
            anim: None,
            drag: None,
            release: None,
            ctx: RenderContext {
                now_s: 0,
                hour: 0,
                minute: 0,
            },
            layers: HashMap::new(),
            renders: 0,
            dirty: true,
        }
    }

    /// Changes the rendered size (layers are dropped).
    pub fn with_size(mut self, width: u32, height: u32) -> Timeline {
        self.width = width;
        self.height = height;
        self.layers.clear();
        self.dirty = true;
        self
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    pub fn home_index(&self) -> usize {
        self.index_of(self.home_id).expect("home card is present")
    }

    fn index_of(&self, id: CardId) -> Option<usize> {
        self.cards.iter().position(|c| c.id == id)
    }

    fn last(&self) -> usize {
        self.cards.len() - 1
    }

    /// The card the timeline is on or is heading to.
    pub fn current_index(&self) -> usize {
        if self.drag.is_some() {
            (self.pos.round().max(0.0) as usize).min(self.last())
        } else {
            self.target
        }
    }

    pub fn current_card(&self) -> &Card {
        &self.cards[self.current_index()]
    }

    /// The scroll position in card units.
    pub fn position(&self) -> f32 {
        self.pos
    }

    /// True when not dragging and not animating.
    pub fn is_settled(&self) -> bool {
        self.drag.is_none() && self.anim.is_none()
    }

    /// True when the next [`Timeline::render`] would draw a different frame.
    pub fn needs_redraw(&self) -> bool {
        self.dirty
    }

    /// How many times a card was rendered into a layer.
    pub fn render_count(&self) -> u64 {
        self.renders
    }

    /// How many card layers are cached.
    pub fn cached_layers(&self) -> usize {
        self.layers.len()
    }

    /// Sets the time used by cards (clock, relative times). Only cards whose rendering
    /// changes are redrawn.
    pub fn set_context(&mut self, ctx: RenderContext) {
        if ctx == self.ctx {
            return;
        }
        self.ctx = ctx;
        let stale = self.layers.iter().any(|(id, layer)| {
            self.index_of(*id)
                .is_some_and(|i| render_key(&self.cards[i], &ctx) != layer.key)
        });
        if stale {
            self.dirty = true;
        }
    }

    // ---- model changes ----

    /// Adds a card at the newest end of the history (next to home). The card on screen
    /// stays on screen.
    pub fn insert_history(&mut self, card: Card) {
        let at = self.home_index() + 1;
        self.insert_at(at, card);
    }

    /// Adds a card on the backward side, next to home. The card on screen stays on screen.
    pub fn insert_backward(&mut self, card: Card) {
        let at = self.home_index();
        self.insert_at(at, card);
    }

    fn insert_at(&mut self, at: usize, card: Card) {
        self.cards.insert(at, card);
        let shift = |i: usize| if i >= at { i + 1 } else { i };
        self.target = shift(self.target);
        let moves = self.pos >= at as f32 - 0.5;
        if moves {
            self.pos += 1.0;
            if let Some(a) = &mut self.anim {
                a.from += 1.0;
                a.to += 1.0;
            }
            if let Some(d) = &mut self.drag {
                d.start_pos += 1.0;
                d.start_index = shift(d.start_index);
            }
        }
        self.dirty = true;
    }

    /// Replaces the content of card `id`. Returns false if there is no such card.
    pub fn update_card(&mut self, id: CardId, content: CardContent) -> bool {
        match self.index_of(id) {
            Some(i) => {
                self.cards[i].content = content;
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    /// Removes card `id` (not the home card). Returns false if nothing was removed.
    pub fn remove_card(&mut self, id: CardId) -> bool {
        if id == self.home_id {
            return false;
        }
        let Some(at) = self.index_of(id) else {
            return false;
        };
        self.cards.remove(at);
        self.layers.remove(&id);
        // Stay on the same card; if it was the removed one, show its older neighbour (or
        // the newer one at the end). Motion stops: the layout under the finger changed.
        if self.target > at {
            self.target -= 1;
        }
        self.target = self.target.min(self.last());
        self.drag = None;
        self.release = None;
        self.anim = None;
        self.pos = self.target as f32;
        self.dirty = true;
        true
    }

    /// Animates back to the home card.
    pub fn go_home(&mut self) {
        let home = self.home_index();
        self.drag = None;
        self.animate_to(home);
    }

    // ---- input ----

    /// Handles one input event and says what the launcher should do, if anything.
    pub fn handle(&mut self, event: &InputEvent) -> Option<Action> {
        match *event {
            InputEvent::Scroll {
                displacement,
                velocity,
                ended,
                cancelled,
                ..
            } => {
                self.on_scroll(displacement, velocity, ended, cancelled);
                None
            }
            InputEvent::Gesture { kind, .. } => self.on_gesture(kind),
        }
    }

    fn on_scroll(&mut self, displacement: f32, velocity: f32, ended: bool, cancelled: bool) {
        let drag = *self.drag.get_or_insert(Drag {
            start_pos: self.pos,
            start_index: self.target,
        });
        self.anim = None;
        self.release = None;
        self.pos = self.rubber_band(drag.start_pos + displacement * CARDS_PER_PAD_WIDTH);
        self.dirty = true;
        if !ended {
            return;
        }
        self.drag = None;
        if cancelled {
            self.animate_to(drag.start_index);
            return;
        }
        let v = velocity * CARDS_PER_PAD_WIDTH;
        let projected = self.pos + v * FLING_PROJECTION_S;
        let mut to = self.clamp_index(projected.round());
        if velocity.abs() >= FLING_MIN_VELOCITY && to == drag.start_index {
            to = self.step(drag.start_index, velocity > 0.0);
        }
        self.release = Some(Release {
            start_index: drag.start_index,
            age: 0.0,
        });
        self.animate_to(to);
    }

    fn on_gesture(&mut self, kind: GestureKind) -> Option<Action> {
        let release = self.release.take();
        match kind {
            GestureKind::SwipeForward | GestureKind::SwipeBackward => {
                let forward = kind == GestureKind::SwipeForward;
                if self.drag.is_some() {
                    return None;
                }
                let to = match release {
                    // The swipe that ended the drag: at least one card from the drag start.
                    Some(r) => {
                        let min_step = self.step(r.start_index, forward);
                        if forward {
                            self.target.max(min_step)
                        } else {
                            self.target.min(min_step)
                        }
                    }
                    None => self.step(self.target, forward),
                };
                if to != self.target {
                    self.animate_to(to);
                }
                None
            }
            GestureKind::Tap => {
                if self.drag.is_some() {
                    return None;
                }
                Some(Action::OpenMenu(self.current_card().id))
            }
            GestureKind::SwipeDown => {
                if self.current_index() == self.home_index() {
                    Some(Action::RequestSleep)
                } else {
                    self.go_home();
                    Some(Action::Back)
                }
            }
            GestureKind::LongPress | GestureKind::SwipeUp => None,
        }
    }

    fn step(&self, from: usize, forward: bool) -> usize {
        if forward {
            (from + 1).min(self.last())
        } else {
            from.saturating_sub(1)
        }
    }

    fn clamp_index(&self, p: f32) -> usize {
        (p.max(0.0) as usize).min(self.last())
    }

    fn rubber_band(&self, p: f32) -> f32 {
        let max = self.last() as f32;
        if p < 0.0 {
            -((-p) * EDGE_RESISTANCE).min(MAX_OVERSCROLL)
        } else if p > max {
            max + ((p - max) * EDGE_RESISTANCE).min(MAX_OVERSCROLL)
        } else {
            p
        }
    }

    fn animate_to(&mut self, to: usize) {
        self.target = to;
        let distance = (to as f32 - self.pos).abs();
        if distance == 0.0 {
            self.anim = None;
            return;
        }
        self.anim = Some(Animation {
            from: self.pos,
            to: to as f32,
            elapsed: 0.0,
            duration: (0.12 + 0.12 * distance).min(0.45),
        });
        self.dirty = true;
    }

    /// Advances time by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        if let Some(r) = &mut self.release {
            r.age += dt;
            if r.age > SWIPE_AFTER_SCROLL_S {
                self.release = None;
            }
        }
        if let Some(a) = &mut self.anim {
            a.elapsed += dt;
            let t = (a.elapsed / a.duration).min(1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            self.pos = if t >= 1.0 {
                a.to
            } else {
                a.from + (a.to - a.from) * eased
            };
            if t >= 1.0 {
                self.anim = None;
            }
            self.dirty = true;
        }
    }

    // ---- rendering ----

    fn ensure_layer(&mut self, index: usize) {
        let card = &self.cards[index];
        let key = render_key(card, &self.ctx);
        if self.layers.get(&card.id).is_some_and(|l| l.key == key) {
            return;
        }
        let frame = render_card(card, &self.ctx, self.width, self.height);
        self.renders += 1;
        self.layers.insert(card.id, Layer { key, frame });
    }

    /// Composites the visible cards at the current position into `frame` (which must be
    /// the timeline's size). At rest, also renders the neighbouring cards ahead and drops
    /// layers that are no longer near.
    pub fn render(&mut self, frame: &mut Frame) {
        let w = self.width as i32;
        let base = self.pos.floor();
        let first = base as i64;
        let offset = ((self.pos - base) * w as f32).round() as i32;
        let mut visible: Vec<(i64, i32)> = vec![(first, -offset)];
        if offset != 0 {
            visible.push((first + 1, w - offset));
        }
        let covered = visible
            .iter()
            .all(|&(i, _)| i >= 0 && (i as usize) < self.cards.len());
        if !covered {
            frame.fill(Color::BLACK);
        }
        for &(i, x) in &visible {
            if i < 0 || i as usize >= self.cards.len() {
                continue;
            }
            let i = i as usize;
            self.ensure_layer(i);
            let layer = &self.layers[&self.cards[i].id].frame;
            frame.copy_from(layer, x, 0);
        }
        if self.is_settled() {
            self.prepare_neighbours();
        }
        self.dirty = false;
    }

    fn prepare_neighbours(&mut self) {
        let c = self.target;
        let lo = c.saturating_sub(1);
        let hi = (c + 1).min(self.last());
        for i in lo..=hi {
            self.ensure_layer(i);
        }
        let keep: Vec<CardId> = self.cards[lo..=hi].iter().map(|c| c.id).collect();
        self.layers.retain(|id, _| keep.contains(id));
    }
}
