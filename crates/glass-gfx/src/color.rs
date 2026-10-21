/// A colour with straight (not premultiplied) alpha, 8 bits per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    /// An opaque grey.
    pub const fn grey(v: u8) -> Color {
        Color::rgb(v, v, v)
    }

    pub(crate) fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    /// Premultiplied channels.
    pub fn premultiplied(self) -> [u8; 4] {
        let a = self.a as u32;
        [
            mul255(self.r as u32, a) as u8,
            mul255(self.g as u32, a) as u8,
            mul255(self.b as u32, a) as u8,
            self.a,
        ]
    }

    pub(crate) fn from_premultiplied(p: [u8; 4]) -> Color {
        let a = p[3] as u32;
        if a == 0 {
            return Color::TRANSPARENT;
        }
        if a == 255 {
            return Color::rgb(p[0], p[1], p[2]);
        }
        let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
        Color::rgba(un(p[0]), un(p[1]), un(p[2]), p[3])
    }
}

/// `a * b / 255`, rounded, for `a, b` in `0..=255`.
#[inline]
pub(crate) fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}
