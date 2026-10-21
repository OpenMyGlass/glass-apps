//! The bundled Inter font (SIL Open Font License 1.1, see `assets/fonts/`).

use fontdue::{Font, FontSettings};
use std::sync::OnceLock;

/// A bundled font weight. Only Light is bundled today; other weights of Inter can be added
/// to `assets/fonts/` when the UI uses them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Weight {
    Light,
}

static LIGHT: &[u8] = include_bytes!("../../../assets/fonts/Inter-Light.ttf");

/// The bundled font of weight `w`, parsed once.
pub fn font(w: Weight) -> &'static Font {
    static LIGHT_FONT: OnceLock<Font> = OnceLock::new();
    match w {
        Weight::Light => LIGHT_FONT.get_or_init(|| {
            Font::from_bytes(LIGHT, FontSettings::default()).expect("bundled font parses")
        }),
    }
}
