//! The bundled Inter fonts (SIL Open Font License 1.1, see `assets/fonts/`).

use fontdue::{Font, FontSettings};
use std::sync::OnceLock;

/// A bundled font weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Thin,
    Light,
    Regular,
}

static THIN: &[u8] = include_bytes!("../../../assets/fonts/Inter-Thin.ttf");
static LIGHT: &[u8] = include_bytes!("../../../assets/fonts/Inter-Light.ttf");
static REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

fn load(bytes: &'static [u8]) -> Font {
    Font::from_bytes(bytes, FontSettings::default()).expect("bundled font parses")
}

/// The bundled font of weight `w`, parsed once.
pub fn font(w: Weight) -> &'static Font {
    static CELLS: [OnceLock<Font>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    match w {
        Weight::Thin => CELLS[0].get_or_init(|| load(THIN)),
        Weight::Light => CELLS[1].get_or_init(|| load(LIGHT)),
        Weight::Regular => CELLS[2].get_or_init(|| load(REGULAR)),
    }
}
