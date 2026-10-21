//! CPU downscaling of images before upload: NPOT textures have no mipmaps, so a texture is
//! uploaded at about the size it is drawn, and photos larger than `GL_MAX_TEXTURE_SIZE`
//! become drawable.

use glass_gfx::Frame;

/// The texture size for drawing an `iw`×`ih` image into a `dw`×`dh` rectangle: the drawn
/// size (rounded up) but never larger than the image, then shrunk, keeping the aspect
/// ratio, to fit `max`.
pub fn texture_size(iw: u32, ih: u32, dw: f32, dh: f32, max: u32) -> (u32, u32) {
    let fit = |drawn: f32, image: u32| (drawn.ceil().max(1.0) as u32).min(image).max(1);
    let (mut w, mut h) = (fit(dw, iw), fit(dh, ih));
    if w > max || h > max {
        let s = max as f64 / w.max(h) as f64;
        w = ((w as f64 * s).floor() as u32).clamp(1, max);
        h = ((h as f64 * s).floor() as u32).clamp(1, max);
    }
    (w, h)
}

/// `src` shrunk to `w`×`h` (each no larger than the source) by averaging the source pixels
/// that fall in each destination pixel. Works on premultiplied pixels, which is what makes
/// averaging correct at transparent edges.
pub fn downscale(src: &Frame, w: u32, h: u32) -> Frame {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (w, h) = (
        w.clamp(1, sw as u32) as usize,
        h.clamp(1, sh as u32) as usize,
    );
    let mut out = Frame::new(w as u32, h as u32);
    let s = src.data();
    let d = out.data_mut();
    let span = |i: usize, n: usize, sn: usize| {
        let a = i * sn / n;
        let b = ((i + 1) * sn / n).max(a + 1);
        (a, b)
    };
    let xs: Vec<(usize, usize)> = (0..w).map(|x| span(x, w, sw)).collect();
    for y in 0..h {
        let (y0, y1) = span(y, h, sh);
        for (x, &(x0, x1)) in xs.iter().enumerate() {
            let mut acc = [0u32; 4];
            for sy in y0..y1 {
                let row = &s[(sy * sw + x0) * 4..(sy * sw + x1) * 4];
                for px in row.chunks_exact(4) {
                    for c in 0..4 {
                        acc[c] += px[c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (y * w + x) * 4;
            for c in 0..4 {
                d[o + c] = ((acc[c] + n / 2) / n) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glass_gfx::Color;

    #[test]
    fn texture_size_follows_the_drawn_size_and_the_limit() {
        // A 4000x3000 photo drawn at 640x480: uploaded at 640x480.
        assert_eq!(texture_size(4000, 3000, 640.0, 480.0, 2048), (640, 480));
        // Drawn larger than the image: the image size, no CPU upscaling.
        assert_eq!(texture_size(100, 50, 640.0, 320.0, 2048), (100, 50));
        // Drawn larger than the limit: shrunk to fit it, aspect kept.
        assert_eq!(texture_size(8000, 4000, 4000.0, 2000.0, 2048), (2048, 1024));
        assert_eq!(texture_size(10, 10, 0.2, 0.2, 2048), (1, 1));
    }

    #[test]
    fn downscale_averages_blocks() {
        let mut f = Frame::new(4, 2);
        f.fill(Color::BLACK);
        // Left 2x2 block white, right block black.
        for y in 0..2 {
            for x in 0..2 {
                let i = (y * 4 + x) * 4;
                f.data_mut()[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let d = downscale(&f, 2, 1);
        assert_eq!(d.pixel(0, 0), Color::WHITE);
        assert_eq!(d.pixel(1, 0), Color::BLACK);
        let g = downscale(&f, 1, 1);
        assert_eq!(g.pixel(0, 0), Color::grey(128));
    }

    #[test]
    fn downscale_to_the_same_size_is_a_copy() {
        let mut f = Frame::new(3, 3);
        f.fill(Color::rgba(10, 20, 30, 128));
        assert_eq!(downscale(&f, 3, 3).data(), f.data());
    }
}
