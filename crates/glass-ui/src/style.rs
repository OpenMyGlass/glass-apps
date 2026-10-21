//! Card style constants, after the public Glass design guidelines: full-bleed black cards,
//! white light text with generous margins, a footer of smaller grey text at the bottom.

use glass_gfx::Color;

/// Left, right and top margin of card content, in pixels.
pub const MARGIN: f32 = 40.0;
/// Main text colour.
pub const TEXT: Color = Color::WHITE;
/// Footer and secondary text colour.
pub const SECONDARY: Color = Color::grey(0x80);
/// Card background.
pub const BACKGROUND: Color = Color::BLACK;

/// Size of the home clock.
pub const CLOCK_SIZE: f32 = 128.0;
/// Body text sizes, tried from the largest down until the text fits (Glass picks the text
/// size from the amount of text).
pub const BODY_SIZES: [f32; 4] = [64.0, 48.0, 40.0, 34.0];
/// Body line height as a multiple of the text size.
pub const BODY_LINE_HEIGHT: f32 = 1.2;
/// Footer text size.
pub const FOOTER_SIZE: f32 = 26.0;
/// Footer baseline, measured up from the bottom edge.
pub const FOOTER_BASELINE_FROM_BOTTOM: f32 = 30.0;
/// Space kept free above the footer baseline for body text.
pub const FOOTER_HEIGHT: f32 = 64.0;
/// Settings card title size.
pub const TITLE_SIZE: f32 = 64.0;
/// Settings card subtitle size.
pub const SUBTITLE_SIZE: f32 = 32.0;
/// Image placeholder fill.
pub const PLACEHOLDER: Color = Color::grey(0x26);
