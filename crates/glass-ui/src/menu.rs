//! Menus and adjustment sub-screens, as overlays over the timeline.
//!
//! - [`Menu`]: the items of a card's menu, one per screen as on Glass (large text, no
//!   icons), moved through with `SWIPE_FORWARD`/`SWIPE_BACKWARD` and followed live with
//!   `Scroll` (the timeline's motion model); `TAP` selects, `SWIPE_DOWN` closes.
//! - [`Adjuster`]: a value in a range (the head-wake angle): swipes step it, `TAP` keeps it,
//!   `SWIPE_DOWN` cancels.
//!
//! Both render through a [`Compositor`] like the timeline (each keeps its own small
//! [`Timeline`] of cards, so layers, the motion and `tick` work the same way).

use crate::card::{Card, CardContent, RenderContext};
use crate::input::{GestureKind, InputEvent};
use crate::timeline::{Action, Timeline};
use glass_gfx::{Compositor, GfxError};

/// An entry of a [`Menu`]: what it selects and how it reads.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem<T> {
    pub value: T,
    pub label: String,
    /// A grey line under the label ("current", "on").
    pub detail: Option<String>,
}

impl<T> MenuItem<T> {
    pub fn new(value: T, label: impl Into<String>) -> MenuItem<T> {
        MenuItem {
            value,
            label: label.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> MenuItem<T> {
        self.detail = Some(detail.into());
        self
    }
}

/// What a menu or an adjuster did with an input event.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome<T> {
    /// `TAP`: this was chosen; the overlay is done.
    Selected(T),
    /// `SWIPE_DOWN`: closed without a choice.
    Closed,
}

/// See the module documentation.
pub struct Menu<T> {
    items: Vec<MenuItem<T>>,
    strip: Timeline,
}

fn item_card<T>(i: usize, item: &MenuItem<T>, count: usize) -> Card {
    Card::new(
        i as u64,
        CardContent::MenuItem {
            label: item.label.clone(),
            detail: item.detail.clone(),
            index: i,
            count,
        },
    )
}

impl<T: Clone> Menu<T> {
    /// A menu showing its first item. `None` when there are no items (a card without a
    /// menu).
    pub fn new(items: Vec<MenuItem<T>>) -> Option<Menu<T>> {
        let n = items.len();
        let mut cards = items.iter().enumerate().map(|(i, it)| item_card(i, it, n));
        let first = cards.next()?;
        let strip = Timeline::new(vec![], first, cards.collect());
        Some(Menu { items, strip })
    }

    /// Changes the size rendered (default: the Glass display).
    pub fn with_size(mut self, width: u32, height: u32) -> Menu<T> {
        self.strip = self.strip.with_size(width, height);
        self
    }

    pub fn items(&self) -> &[MenuItem<T>] {
        &self.items
    }

    /// The item shown (or being moved to).
    pub fn index(&self) -> usize {
        self.strip.current_index()
    }

    /// Shows item `index` at once.
    pub fn select_index(&mut self, index: usize) {
        self.strip.show_index(index);
    }

    /// Handles one input event.
    pub fn handle(&mut self, event: &InputEvent) -> Option<Outcome<T>> {
        match *event {
            InputEvent::Gesture { fingers, .. } if fingers != 1 => None,
            InputEvent::Gesture {
                kind: GestureKind::SwipeDown,
                ..
            } => Some(Outcome::Closed),
            InputEvent::Gesture {
                kind: GestureKind::LongPress | GestureKind::SwipeUp,
                ..
            } => None,
            _ => match self.strip.handle(event) {
                Some(Action::OpenMenu(i)) => {
                    Some(Outcome::Selected(self.items[i as usize].value.clone()))
                }
                _ => None,
            },
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.strip.tick(dt);
    }

    /// Ends a touch whose end will not arrive (focus lost, glassd gone): a drag returns to
    /// the item it started from.
    pub fn cancel_touch(&mut self) {
        self.strip.cancel_touch();
    }

    pub fn is_settled(&self) -> bool {
        self.strip.is_settled()
    }

    pub fn needs_redraw(&self) -> bool {
        self.strip.needs_redraw()
    }

    /// Composes the menu (the item shown, or two items while moving).
    pub fn render(&mut self, comp: &mut dyn Compositor) -> Result<(), GfxError> {
        self.strip.render(comp)
    }

    /// Frees the menu's layers (call when it closes).
    pub fn release(&mut self, comp: &mut dyn Compositor) {
        self.strip.release_layers(comp);
    }

    /// The timeline of item cards (tests: render counts, positions).
    pub fn strip(&self) -> &Timeline {
        &self.strip
    }
}

/// A value adjusted in steps within a range; see the module documentation.
pub struct Adjuster {
    title: String,
    min: f32,
    max: f32,
    step: f32,
    value: f32,
    format: fn(f32) -> String,
    screen: Timeline,
}

/// `v` clamped to `min..=max` and moved to the nearest step from `min`.
fn snap(v: f32, min: f32, max: f32, step: f32) -> f32 {
    let v = v.clamp(min, max);
    if step <= 0.0 {
        return v;
    }
    (min + ((v - min) / step).round() * step).clamp(min, max)
}

impl Adjuster {
    /// `value` starts clamped to `min..=max` and snapped to the step grid from `min`;
    /// `format` turns it into the text shown.
    pub fn new(
        title: impl Into<String>,
        min: f32,
        max: f32,
        step: f32,
        value: f32,
        format: fn(f32) -> String,
    ) -> Adjuster {
        let mut a = Adjuster {
            title: title.into(),
            min,
            max,
            step,
            value: snap(value, min, max, step),
            format,
            screen: Timeline::new(vec![], Card::new(0, CardContent::Home), vec![]),
        };
        a.screen = Timeline::new(vec![], a.card(), vec![]);
        a
    }

    fn card(&self) -> Card {
        let span = self.max - self.min;
        Card::new(
            0,
            CardContent::Adjust {
                title: self.title.clone(),
                value: (self.format)(self.value),
                fraction: if span > 0.0 {
                    (self.value - self.min) / span
                } else {
                    1.0
                },
            },
        )
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    fn set(&mut self, v: f32) {
        let v = v.clamp(self.min, self.max);
        if v != self.value {
            self.value = v;
            self.screen.update_card(0, self.card().content);
        }
    }

    /// Handles one input event: forward raises the value one step, backward lowers it.
    pub fn handle(&mut self, event: &InputEvent) -> Option<Outcome<f32>> {
        let InputEvent::Gesture { kind, fingers, .. } = *event else {
            return None;
        };
        if fingers != 1 {
            return None;
        }
        match kind {
            GestureKind::SwipeForward => self.set(self.value + self.step),
            GestureKind::SwipeBackward => self.set(self.value - self.step),
            GestureKind::Tap => return Some(Outcome::Selected(self.value)),
            GestureKind::SwipeDown => return Some(Outcome::Closed),
            GestureKind::LongPress | GestureKind::SwipeUp => {}
        }
        None
    }

    pub fn needs_redraw(&self) -> bool {
        self.screen.needs_redraw()
    }

    pub fn set_context(&mut self, ctx: RenderContext) {
        self.screen.set_context(ctx);
    }

    pub fn render(&mut self, comp: &mut dyn Compositor) -> Result<(), GfxError> {
        self.screen.render(comp)
    }

    pub fn release(&mut self, comp: &mut dyn Compositor) {
        self.screen.release_layers(comp);
    }
}
