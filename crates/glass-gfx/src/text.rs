//! Text measurement, layout and drawing with `fontdue`.
//!
//! Layout ([`layout_line`], [`layout_box_runs`]) is separate from drawing: it produces
//! [`GlyphRun`]s at whole-pixel positions, which any [`crate::Painter`] draws.
//!
//! Sizes are in pixels (the font's em size). Glyphs are placed at whole pixels on the pen
//! position (rounded), which keeps text crisp and the output identical on every platform.
//! Kerning from the font's `kern` table is applied. There is no shaping beyond that, which
//! is enough for Latin text in the card UI.

use crate::{Color, Font, Frame, Rect};

/// The ellipsis appended to text that does not fit.
pub const ELLIPSIS: char = '…';

/// Horizontal alignment of lines inside a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Ascent (positive, above the baseline) and descent (positive, below it) at `size` px.
pub fn line_metrics(font: &Font, size: f32) -> (f32, f32) {
    match font.horizontal_line_metrics(size) {
        Some(m) => (m.ascent, -m.descent),
        None => (size * 0.8, size * 0.2),
    }
}

/// Pen positions of each character relative to the start, and the total advance.
fn pen_positions(font: &Font, size: f32, text: &str) -> (Vec<(char, f32)>, f32) {
    let mut out = Vec::with_capacity(text.len());
    let mut pen = 0.0f32;
    let mut prev: Option<char> = None;
    for ch in text.chars() {
        if let Some(p) = prev {
            pen += font.horizontal_kern(p, ch, size).unwrap_or(0.0);
        }
        out.push((ch, pen));
        pen += font.metrics(ch, size).advance_width;
        prev = Some(ch);
    }
    (out, pen)
}

/// The advance width of `text` on one line, in pixels.
pub fn measure(font: &Font, size: f32, text: &str) -> f32 {
    pen_positions(font, size, text).1
}

/// A glyph placed on a whole pixel: `x` is the pen position (the glyph's origin) and
/// `baseline` the baseline row. The rasterized bitmap's top-left corner is at
/// `(x + metrics.xmin, baseline - (metrics.ymin + metrics.height))`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionedGlyph {
    pub ch: char,
    pub x: i32,
    pub baseline: i32,
}

/// Glyphs of one font and size, laid out at whole-pixel positions. Both painters draw
/// these, so the software and GLES paths place every glyph on the same pixel.
#[derive(Clone, Debug)]
pub struct GlyphRun<'a> {
    pub font: &'a Font,
    pub size: f32,
    pub glyphs: Vec<PositionedGlyph>,
}

impl GlyphRun<'_> {
    /// Where glyph `g`'s bitmap goes and its size: `(x, y, width, height)`.
    pub fn bitmap_rect(&self, g: &PositionedGlyph) -> (i32, i32, usize, usize) {
        let m = self.font.metrics(g.ch, self.size);
        (
            g.x + m.xmin,
            g.baseline - (m.ymin + m.height as i32),
            m.width,
            m.height,
        )
    }
}

/// Lays out `text` on one line with its start at `x` and its baseline at `baseline`.
/// Whitespace produces no glyph. Returns the run and the advance width.
pub fn layout_line<'a>(
    font: &'a Font,
    size: f32,
    x: f32,
    baseline: f32,
    text: &str,
) -> (GlyphRun<'a>, f32) {
    let (pens, width) = pen_positions(font, size, text);
    let base = baseline.round() as i32;
    let glyphs = pens
        .into_iter()
        .filter(|(ch, _)| !ch.is_whitespace())
        .map(|(ch, pen)| PositionedGlyph {
            ch,
            x: (x + pen).round() as i32,
            baseline: base,
        })
        .collect();
    (GlyphRun { font, size, glyphs }, width)
}

/// Draws `text` on one line with its start at `x` and its baseline at `baseline`.
/// Returns the advance width.
pub fn draw_line(
    frame: &mut Frame,
    font: &Font,
    size: f32,
    x: f32,
    baseline: f32,
    text: &str,
    color: Color,
) -> f32 {
    let (run, width) = layout_line(font, size, x, baseline, text);
    frame.draw_glyph_run(&run, color);
    width
}

/// `text` shortened with a trailing ellipsis so that it fits in `max_width`; `text` itself
/// if it already fits; an empty string if not even the ellipsis fits.
pub fn ellipsize(font: &Font, size: f32, text: &str, max_width: f32) -> String {
    if measure(font, size, text) <= max_width {
        return text.to_string();
    }
    truncate_with_ellipsis(font, size, text, max_width)
}

/// The longest prefix of `text` followed by an ellipsis that fits in `max_width`.
fn truncate_with_ellipsis(font: &Font, size: f32, text: &str, max_width: f32) -> String {
    let ends: Vec<usize> = text.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
    // Longest prefix first; text lengths on a card are small, so a linear scan is fine.
    for &end in ends.iter().rev() {
        let candidate = format!("{}{}", text[..end].trim_end(), ELLIPSIS);
        if measure(font, size, &candidate) <= max_width {
            return candidate;
        }
    }
    let lone = ELLIPSIS.to_string();
    if measure(font, size, &lone) <= max_width {
        lone
    } else {
        String::new()
    }
}

/// Breaks `text` into lines no wider than `max_width`: at spaces when possible, inside a
/// word when the word alone is too wide. Explicit `\n` starts a new line.
pub fn wrap(font: &Font, size: f32, text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if measure(font, size, &candidate) <= max_width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            if measure(font, size, word) <= max_width {
                line = word.to_string();
                continue;
            }
            // Break the word itself.
            for ch in word.chars() {
                let mut next = line.clone();
                next.push(ch);
                if !line.is_empty() && measure(font, size, &next) > max_width {
                    lines.push(std::mem::take(&mut line));
                    line.push(ch);
                } else {
                    line = next;
                }
            }
        }
        lines.push(line);
    }
    lines
}

/// [`wrap`] limited to `max_lines`; when text is cut, the last line ends with an ellipsis.
pub fn layout_box(
    font: &Font,
    size: f32,
    text: &str,
    max_width: f32,
    max_lines: usize,
) -> Vec<String> {
    let mut lines = wrap(font, size, text, max_width);
    if max_lines == 0 {
        return Vec::new();
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        let last = lines.pop().unwrap_or_default();
        lines.push(truncate_with_ellipsis(font, size, &last, max_width));
    }
    lines
}

/// Lays out `text` in `rect` (at most `max_lines`, and no more than fit in its height), the
/// first baseline one ascent below the top, then every `size * line_height`. One run per
/// line.
pub fn layout_box_runs<'a>(
    font: &'a Font,
    size: f32,
    line_height: f32,
    rect: Rect,
    text: &str,
    align: Align,
    max_lines: usize,
) -> Vec<GlyphRun<'a>> {
    let (ascent, descent) = line_metrics(font, size);
    let step = size * line_height;
    let fit = if step <= 0.0 || !step.is_finite() {
        max_lines.min(1)
    } else if rect.h < ascent + descent {
        0
    } else {
        1 + ((rect.h - ascent - descent) / step).floor() as usize
    };
    let lines = layout_box(font, size, text, rect.w, max_lines.min(fit));
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let w = measure(font, size, line);
            let x = match align {
                Align::Left => rect.x,
                Align::Center => rect.x + (rect.w - w) / 2.0,
                Align::Right => rect.right() - w,
            };
            layout_line(font, size, x, rect.y + ascent + step * i as f32, line).0
        })
        .collect()
}

/// [`layout_box_runs`] drawn into `frame`. Returns the number of lines drawn.
#[allow(clippy::too_many_arguments)]
pub fn draw_box(
    frame: &mut Frame,
    font: &Font,
    size: f32,
    line_height: f32,
    rect: Rect,
    text: &str,
    color: Color,
    align: Align,
    max_lines: usize,
) -> usize {
    let runs = layout_box_runs(font, size, line_height, rect, text, align, max_lines);
    for run in &runs {
        frame.draw_glyph_run(run, color);
    }
    runs.len()
}
