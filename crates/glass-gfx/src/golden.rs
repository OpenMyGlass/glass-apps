//! Golden-image comparison for tests: a rendered [`Frame`] against a PNG in a golden
//! directory (or another frame), with a per-scene tolerance. `GLASS_UPDATE_GOLDEN=1`
//! writes the goldens instead (software renderer only: GLES output is never a reference).

use crate::{Frame, Rect};
use std::path::Path;

/// How far a frame may differ from its reference.
#[derive(Clone, Debug, PartialEq)]
pub enum Tolerance {
    /// Every channel of every pixel within this much.
    Max(u8),
    /// The GPU plan's rule for anti-aliased edges drawn by a different method: within ±2
    /// except on at most 1 % of the pixels, which stay within 64.
    Edges,
    /// [`Tolerance::Edges`] only inside these rectangles (each grown by 1 px for the
    /// anti-aliased fringe), within ±2 everywhere else.
    Shapes(Vec<Rect>),
}

/// How two frames differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diff {
    /// Largest channel difference.
    pub worst: u8,
    /// Pixels with a channel more than 2 apart.
    pub pixels_over_2: usize,
    pub pixels: usize,
    /// Largest channel difference outside the shape rectangles ([`Tolerance::Shapes`];
    /// equal to `worst` otherwise).
    pub worst_outside: u8,
}

fn inside(rects: &[Rect], x: u32, y: u32) -> bool {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    rects.iter().any(|r| {
        px >= r.x - 1.0 && px < r.right() + 1.0 && py >= r.y - 1.0 && py < r.bottom() + 1.0
    })
}

/// Compares two frames of the same size; `shapes` are the rectangles where edge pixels
/// may differ more. Panics if the sizes differ.
pub fn diff_with(a: &Frame, b: &Frame, shapes: &[Rect]) -> Diff {
    assert_eq!(
        (a.width(), a.height()),
        (b.width(), b.height()),
        "frame sizes differ"
    );
    let w = a.width();
    let (mut worst, mut worst_outside, mut over) = (0u8, 0u8, 0usize);
    for (i, (pa, pb)) in a
        .data()
        .chunks_exact(4)
        .zip(b.data().chunks_exact(4))
        .enumerate()
    {
        let d = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        worst = worst.max(d);
        if d > 2 {
            over += 1;
        }
        if d > worst_outside && !inside(shapes, i as u32 % w, i as u32 / w) {
            worst_outside = d;
        }
    }
    Diff {
        worst,
        pixels_over_2: over,
        pixels: (a.width() * a.height()) as usize,
        worst_outside,
    }
}

/// [`diff_with`] without shape rectangles.
pub fn diff(a: &Frame, b: &Frame) -> Diff {
    diff_with(a, b, &[])
}

impl Tolerance {
    fn shapes(&self) -> &[Rect] {
        match self {
            Tolerance::Shapes(r) => r,
            _ => &[],
        }
    }

    pub fn accepts(&self, d: &Diff) -> bool {
        let edges = d.pixels_over_2 * 100 <= d.pixels && d.worst <= 64;
        match self {
            Tolerance::Max(m) => d.worst <= *m,
            Tolerance::Edges => edges,
            Tolerance::Shapes(_) => edges && d.worst_outside <= 2,
        }
    }
}

/// Compares `actual` with `expected`. On a mismatch `actual` is written to
/// `<actual_dir>/<name>.png` and the test panics.
pub fn compare(
    actual_dir: &Path,
    name: &str,
    actual: &Frame,
    expected: &Frame,
    tolerance: &Tolerance,
) -> Diff {
    let d = diff_with(actual, expected, tolerance.shapes());
    if !tolerance.accepts(&d) {
        std::fs::create_dir_all(actual_dir).expect("actual directory");
        let out = actual_dir.join(format!("{name}.png"));
        actual.write_png(&out).expect("write actual");
        panic!(
            "{name}: {d:?} exceeds {tolerance:?}; actual image in {}",
            out.display()
        );
    }
    d
}

/// Checks `frame` against `<golden_dir>/<name>.png` (see [`compare`]). With `update` the
/// golden is written instead.
pub fn check(
    golden_dir: &Path,
    actual_dir: &Path,
    name: &str,
    frame: &Frame,
    tolerance: &Tolerance,
    update: bool,
) -> Diff {
    let path = golden_dir.join(format!("{name}.png"));
    if update {
        std::fs::create_dir_all(golden_dir).expect("golden directory");
        frame.write_png(&path).expect("write golden");
        return diff(frame, frame);
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run with GLASS_UPDATE_GOLDEN=1 to create it",
            path.display()
        )
    });
    let golden = Frame::from_png(&bytes).expect("golden PNG");
    compare(actual_dir, name, frame, &golden, tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    #[test]
    fn shapes_allow_edges_inside_only() {
        let mut a = Frame::new(20, 10);
        a.fill(Color::BLACK);
        let mut b = a.clone();
        // One pixel differs by 40 at (5, 5).
        let i = (5 * 20 + 5) * 4;
        b.data_mut()[i] = 40;
        let inside = Tolerance::Shapes(vec![Rect::new(4.0, 4.0, 2.0, 2.0)]);
        let elsewhere = Tolerance::Shapes(vec![Rect::new(12.0, 2.0, 2.0, 2.0)]);
        let d = diff_with(&a, &b, &[Rect::new(4.0, 4.0, 2.0, 2.0)]);
        assert_eq!(d.worst, 40);
        assert_eq!(d.worst_outside, 0);
        assert!(inside.accepts(&d));
        let d2 = diff_with(&a, &b, &[Rect::new(12.0, 2.0, 2.0, 2.0)]);
        assert!(!elsewhere.accepts(&d2));
        assert!(Tolerance::Edges.accepts(&d2));
        assert!(!Tolerance::Max(2).accepts(&d2));
    }
}
