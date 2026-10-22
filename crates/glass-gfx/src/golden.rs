//! Golden-image comparison for tests: a rendered [`Frame`] against a PNG in a golden
//! directory, with a per-scene tolerance. `GLASS_UPDATE_GOLDEN=1` writes the goldens
//! instead (software renderer only: GLES output is never a reference).

use crate::Frame;
use std::path::Path;

/// How far a frame may differ from its golden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tolerance {
    /// Every channel of every pixel within this much.
    Max(u8),
    /// Within ±2 except on at most 1 % of the pixels, which stay within 64 (anti-aliased
    /// edges drawn by a different method, the GPU plan's rule).
    Edges,
}

/// How two frames differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diff {
    /// Largest channel difference.
    pub worst: u8,
    /// Pixels with a channel more than 2 apart.
    pub pixels_over_2: usize,
    pub pixels: usize,
}

/// Compares two frames of the same size. Panics if the sizes differ.
pub fn diff(a: &Frame, b: &Frame) -> Diff {
    assert_eq!(
        (a.width(), a.height()),
        (b.width(), b.height()),
        "frame sizes differ"
    );
    let mut worst = 0;
    let mut over = 0;
    for (pa, pb) in a.data().chunks_exact(4).zip(b.data().chunks_exact(4)) {
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
    }
    Diff {
        worst,
        pixels_over_2: over,
        pixels: (a.width() * a.height()) as usize,
    }
}

impl Tolerance {
    pub fn accepts(self, d: &Diff) -> bool {
        match self {
            Tolerance::Max(m) => d.worst <= m,
            Tolerance::Edges => d.pixels_over_2 * 100 <= d.pixels && d.worst <= 64,
        }
    }
}

/// Checks `frame` against `<golden_dir>/<name>.png`. On a mismatch the frame is written to
/// `<actual_dir>/<name>.png` and the test panics. With `update` the golden is written.
pub fn check(
    golden_dir: &Path,
    actual_dir: &Path,
    name: &str,
    frame: &Frame,
    tolerance: Tolerance,
    update: bool,
) -> Diff {
    let path = golden_dir.join(format!("{name}.png"));
    if update {
        std::fs::create_dir_all(golden_dir).expect("golden directory");
        frame.write_png(&path).expect("write golden");
        return Diff {
            worst: 0,
            pixels_over_2: 0,
            pixels: 0,
        };
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run with GLASS_UPDATE_GOLDEN=1 to create it",
            path.display()
        )
    });
    let golden = Frame::from_png(&bytes).expect("golden PNG");
    let d = diff(frame, &golden);
    if !tolerance.accepts(&d) {
        std::fs::create_dir_all(actual_dir).expect("actual directory");
        let out = actual_dir.join(format!("{name}.png"));
        frame.write_png(&out).expect("write actual");
        panic!(
            "{name}: {d:?} exceeds {tolerance:?}; actual image in {}",
            out.display()
        );
    }
    d
}
