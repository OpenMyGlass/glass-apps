//! Cards and their rendering.

use crate::style;
use glass_gfx::text::{self, Align};
use glass_gfx::{fonts, Frame, Painter, Rect, SkiaPainter, Weight};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// A stable card identifier, unique within a timeline.
pub type CardId = u64;

/// What a card shows.
#[derive(Clone, Debug)]
pub enum CardContent {
    /// The home card: the current time in large light text.
    Home,
    /// A text item (a message, a notification): body text and a footer with its source and
    /// the time since it arrived.
    Text {
        body: String,
        source: String,
        /// When it arrived, Unix seconds.
        timestamp_s: i64,
    },
    /// An entry of the settings bundle: a title, optionally a grey status line.
    Settings {
        title: String,
        subtitle: Option<String>,
    },
    /// A full-bleed image with a caption in the footer; a placeholder when `image` is `None`.
    Image {
        image: Option<CardImage>,
        caption: String,
        timestamp_s: i64,
    },
    /// One item of a menu, one per screen as on Glass: a large label, optionally a grey
    /// detail line ("current"), and dots for the position (`index` of `count`).
    MenuItem {
        label: String,
        detail: Option<String>,
        index: usize,
        count: usize,
    },
    /// A value being adjusted (a sub-screen): a grey title, the value in large text and a
    /// bar filled to `fraction` (`0.0..=1.0`).
    Adjust {
        title: String,
        value: String,
        fraction: f32,
    },
    /// Facts in short lines under a grey title (the device card).
    Info { title: String, lines: Vec<String> },
}

/// An image shown by a card. `id` names the picture: a new picture needs a new id, which is
/// what tells the timeline to re-render the card (the pixels are not compared).
#[derive(Clone, Debug)]
pub struct CardImage {
    pub id: u64,
    pub frame: Arc<Frame>,
}

impl CardContent {
    /// A settings entry without a subtitle.
    pub fn settings(title: impl Into<String>) -> CardContent {
        CardContent::Settings {
            title: title.into(),
            subtitle: None,
        }
    }
}

/// A card in the timeline.
#[derive(Clone, Debug)]
pub struct Card {
    pub id: CardId,
    pub content: CardContent,
}

impl Card {
    pub fn new(id: CardId, content: CardContent) -> Card {
        Card { id, content }
    }
}

/// What cards need from the outside world to render: the time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderContext {
    /// Now, Unix seconds (for relative times in footers).
    pub now_s: i64,
    /// Local wall-clock hour (0..24) and minute, for the home clock.
    pub hour: u8,
    pub minute: u8,
}

/// The home clock text: 12-hour, no leading zero, no AM/PM ("10:42").
pub fn clock_text(hour: u8, minute: u8) -> String {
    let h = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{h}:{minute:02}")
}

/// Footer time: "just now", "5 minutes ago", "1 hour ago", "yesterday", "3 days ago".
pub fn relative_time(now_s: i64, then_s: i64) -> String {
    let age = now_s.saturating_sub(then_s).max(0);
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    match age {
        0..=59 => "just now".into(),
        60..=3_599 => plural(age / 60, "minute"),
        3_600..=86_399 => plural(age / 3_600, "hour"),
        86_400..=172_799 => "yesterday".into(),
        _ => plural(age / 86_400, "day"),
    }
}

/// A hash of everything a card shows in `ctx`: two calls give the same key exactly when the
/// rendered layer would be the same, so a cached layer is reused while its key holds.
pub fn render_key(card: &Card, ctx: &RenderContext) -> u64 {
    let mut h = DefaultHasher::new();
    card.id.hash(&mut h);
    match &card.content {
        CardContent::Home => {
            0u8.hash(&mut h);
            clock_text(ctx.hour, ctx.minute).hash(&mut h);
        }
        CardContent::Text {
            body,
            source,
            timestamp_s,
        } => {
            1u8.hash(&mut h);
            body.hash(&mut h);
            source.hash(&mut h);
            relative_time(ctx.now_s, *timestamp_s).hash(&mut h);
        }
        CardContent::Settings { title, subtitle } => {
            2u8.hash(&mut h);
            title.hash(&mut h);
            subtitle.hash(&mut h);
        }
        CardContent::Image {
            image,
            caption,
            timestamp_s,
        } => {
            3u8.hash(&mut h);
            image.as_ref().map(|i| i.id).hash(&mut h);
            caption.hash(&mut h);
            relative_time(ctx.now_s, *timestamp_s).hash(&mut h);
        }
        CardContent::MenuItem {
            label,
            detail,
            index,
            count,
        } => {
            4u8.hash(&mut h);
            label.hash(&mut h);
            detail.hash(&mut h);
            index.hash(&mut h);
            count.hash(&mut h);
        }
        CardContent::Adjust {
            title,
            value,
            fraction,
        } => {
            5u8.hash(&mut h);
            title.hash(&mut h);
            value.hash(&mut h);
            fraction.to_bits().hash(&mut h);
        }
        CardContent::Info { title, lines } => {
            6u8.hash(&mut h);
            title.hash(&mut h);
            lines.hash(&mut h);
        }
    }
    h.finish()
}

/// Renders `card` into a new opaque frame of `width`×`height` pixels (software painter).
pub fn render_card(card: &Card, ctx: &RenderContext, width: u32, height: u32) -> Frame {
    let mut f = Frame::new(width, height);
    paint_card(card, ctx, &mut SkiaPainter::new(&mut f));
    f
}

/// Paints `card` over the whole of `f`, which ends up opaque. Written against [`Painter`]
/// only, so every renderer draws the same card.
pub fn paint_card(card: &Card, ctx: &RenderContext, f: &mut dyn Painter) {
    f.clear(style::BACKGROUND);
    let (width, height) = f.size();
    let (w, h) = (width as f32, height as f32);
    match &card.content {
        CardContent::Home => {
            centered_line(
                f,
                Weight::Light,
                style::CLOCK_SIZE,
                &clock_text(ctx.hour, ctx.minute),
                h / 2.0,
                style::TEXT,
            );
        }
        CardContent::Text {
            body,
            source,
            timestamp_s,
        } => {
            body_text(f, body);
            footer(
                f,
                source,
                &relative_time(ctx.now_s, *timestamp_s),
                style::SECONDARY,
            );
        }
        CardContent::Settings { title, subtitle } => {
            let title_center = if subtitle.is_some() {
                h * 0.44
            } else {
                h / 2.0
            };
            centered_line(
                f,
                Weight::Light,
                style::TITLE_SIZE,
                title,
                title_center,
                style::TEXT,
            );
            if let Some(sub) = subtitle {
                centered_line(
                    f,
                    Weight::Light,
                    style::SUBTITLE_SIZE,
                    sub,
                    h * 0.66,
                    style::SECONDARY,
                );
            }
        }
        CardContent::Image {
            image,
            caption,
            timestamp_s,
        } => {
            match image {
                Some(CardImage { frame: img, .. }) => {
                    // Cover: scale to fill the card, cropping the longer side.
                    let s = (w / img.width() as f32).max(h / img.height() as f32);
                    let (iw, ih) = (img.width() as f32 * s, img.height() as f32 * s);
                    f.draw_image(img, Rect::new((w - iw) / 2.0, (h - ih) / 2.0, iw, ih));
                }
                None => {
                    f.clear(style::PLACEHOLDER);
                    // A simple picture-frame mark in the middle.
                    let (bw, bh) = (150.0, 104.0);
                    let r = Rect::new((w - bw) / 2.0, (h - bh) / 2.0 - 20.0, bw, bh);
                    f.fill_rounded_rect(r, 10.0, glass_gfx::Color::grey(0x55));
                    let inner = Rect::new(r.x + 8.0, r.y + 8.0, r.w - 16.0, r.h - 16.0);
                    f.fill_rounded_rect(inner, 4.0, style::PLACEHOLDER);
                }
            }
            footer(
                f,
                caption,
                &relative_time(ctx.now_s, *timestamp_s),
                style::TEXT,
            );
        }
        CardContent::MenuItem {
            label,
            detail,
            index,
            count,
        } => {
            let center = if detail.is_some() { h * 0.44 } else { h / 2.0 };
            centered_line(
                f,
                Weight::Light,
                style::MENU_SIZE,
                label,
                center,
                style::TEXT,
            );
            if let Some(d) = detail {
                centered_line(
                    f,
                    Weight::Light,
                    style::SUBTITLE_SIZE,
                    d,
                    h * 0.66,
                    style::SECONDARY,
                );
            }
            position_dots(f, *index, *count);
        }
        CardContent::Adjust {
            title,
            value,
            fraction,
        } => {
            centered_line(
                f,
                Weight::Light,
                style::SUBTITLE_SIZE,
                title,
                style::MARGIN + 20.0,
                style::SECONDARY,
            );
            centered_line(
                f,
                Weight::Light,
                style::ADJUST_VALUE_SIZE,
                value,
                h * 0.48,
                style::TEXT,
            );
            let track = Rect::new(
                style::MARGIN,
                h - style::FOOTER_BASELINE_FROM_BOTTOM - 20.0,
                w - 2.0 * style::MARGIN,
                style::BAR_HEIGHT,
            );
            let r = style::BAR_HEIGHT / 2.0;
            f.fill_rounded_rect(track, r, style::TRACK);
            let fill = track.w * fraction.clamp(0.0, 1.0);
            if fill > 0.0 {
                f.fill_rounded_rect(
                    Rect::new(track.x, track.y, fill.max(2.0 * r), track.h),
                    r,
                    style::TEXT,
                );
            }
        }
        CardContent::Info { title, lines } => {
            centered_line(
                f,
                Weight::Light,
                style::SUBTITLE_SIZE,
                title,
                style::MARGIN + 20.0,
                style::SECONDARY,
            );
            info_lines(f, lines);
        }
    }
}

/// The info card's lines: left-aligned, wrapped, as many as fit above the bottom margin.
fn info_lines(f: &mut dyn Painter, lines: &[String]) {
    let font = fonts::font(Weight::Light);
    let (w, h) = f.size();
    let size = style::INFO_SIZE;
    let step = size * style::BODY_LINE_HEIGHT;
    let max_w = w as f32 - 2.0 * style::MARGIN;
    let mut baseline = style::INFO_TOP;
    for line in lines.iter().flat_map(|l| text::wrap(font, size, l, max_w)) {
        if baseline > h as f32 - style::FOOTER_BASELINE_FROM_BOTTOM {
            break;
        }
        let (run, _) = text::layout_line(font, size, style::MARGIN, baseline, &line);
        f.draw_glyph_run(&run, style::TEXT);
        baseline += step;
    }
}

/// Dots showing which of `count` menu items is shown, centred near the bottom.
fn position_dots(f: &mut dyn Painter, index: usize, count: usize) {
    if count < 2 {
        return;
    }
    let (w, h) = f.size();
    let d = style::DOT_SIZE;
    let gap = style::DOT_GAP;
    let total = count as f32 * d + (count - 1) as f32 * gap;
    let x0 = ((w as f32 - total) / 2.0).round();
    let y = h as f32 - style::FOOTER_BASELINE_FROM_BOTTOM - d;
    for i in 0..count {
        let c = if i == index {
            style::TEXT
        } else {
            style::TRACK
        };
        let x = x0 + i as f32 * (d + gap);
        f.fill_rounded_rect(Rect::new(x, y, d, d), d / 2.0, c);
    }
}

/// Draws one line centred horizontally, its digits/capitals centred on `center_y`.
fn centered_line(
    f: &mut dyn Painter,
    weight: Weight,
    size: f32,
    line: &str,
    center_y: f32,
    color: glass_gfx::Color,
) {
    let font = fonts::font(weight);
    let (fw, _) = f.size();
    let max_w = fw as f32 - 2.0 * style::MARGIN;
    let line = text::ellipsize(font, size, line, max_w);
    let w = text::measure(font, size, &line);
    // Cap height from the glyph of '0' (same as capitals in Inter).
    let cap = font.metrics('0', size).height as f32;
    let x = (fw as f32 - w) / 2.0;
    let (run, _) = text::layout_line(font, size, x, center_y + cap / 2.0, &line);
    f.draw_glyph_run(&run, color);
}

/// Body text in the content area, at the largest size that fits; the smallest size
/// ellipsizes what does not fit.
fn body_text(f: &mut dyn Painter, body: &str) {
    let font = fonts::font(Weight::Light);
    let (fw, fh) = f.size();
    let area = Rect::new(
        style::MARGIN,
        style::MARGIN - 8.0,
        fw as f32 - 2.0 * style::MARGIN,
        fh as f32 - style::MARGIN - style::FOOTER_HEIGHT,
    );
    let mut chosen = *style::BODY_SIZES.last().unwrap();
    for size in style::BODY_SIZES {
        let lines = text::wrap(font, size, body, area.w).len();
        if lines <= lines_that_fit(font, size, area.h) {
            chosen = size;
            break;
        }
    }
    for run in text::layout_box_runs(
        font,
        chosen,
        style::BODY_LINE_HEIGHT,
        area,
        body,
        Align::Left,
        usize::MAX,
    ) {
        f.draw_glyph_run(&run, style::TEXT);
    }
}

fn lines_that_fit(font: &glass_gfx::Font, size: f32, height: f32) -> usize {
    let (ascent, descent) = text::line_metrics(font, size);
    if height < ascent + descent {
        return 0;
    }
    1 + ((height - ascent - descent) / (size * style::BODY_LINE_HEIGHT)).floor() as usize
}

/// The footer: `left` (source, caption) at the left margin, `right` (time) right-aligned.
fn footer(f: &mut dyn Painter, left: &str, right: &str, left_color: glass_gfx::Color) {
    let font = fonts::font(Weight::Light);
    let size = style::FOOTER_SIZE;
    let (fw, fh) = f.size();
    let baseline = fh as f32 - style::FOOTER_BASELINE_FROM_BOTTOM;
    let right_edge = fw as f32 - style::MARGIN;
    let rw = text::measure(font, size, right);
    let (run, _) = text::layout_line(font, size, right_edge - rw, baseline, right);
    f.draw_glyph_run(&run, style::SECONDARY);
    let room = right_edge - rw - 24.0 - style::MARGIN;
    let left = text::ellipsize(font, size, left, room);
    let (run, _) = text::layout_line(font, size, style::MARGIN, baseline, &left);
    f.draw_glyph_run(&run, left_color);
}
