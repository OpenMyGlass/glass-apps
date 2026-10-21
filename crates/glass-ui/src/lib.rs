//! Glass cards and the card timeline.
//!
//! - [`card`]: the card model and its Glass-style rendering at 640×360.
//! - [`timeline`]: the ordered timeline with the home card in the middle, navigation by
//!   gestures, live dragging by `Scroll`, settle animation, and compositing of cached
//!   per-card layers.
//! - [`input`]: the input events, mirroring the glass-services interface 1:1.

pub mod card;
pub mod input;
pub mod style;
pub mod timeline;

pub use card::{Card, CardContent, CardId, RenderContext};
pub use input::{GestureKind, InputEvent};
pub use timeline::{Action, Timeline, CARDS_PER_PAD_WIDTH};
