//! Frame timing for renderer comparisons, on when `GLASS_FRAME_STATS` is set: one stderr
//! line per burst of animation (frames less than 200 ms apart), so the init can log it.

use std::time::{Duration, Instant};

const BURST_GAP: Duration = Duration::from_millis(200);
const MAX_BURST: usize = 240;

/// Render and present times of the frames of the current burst.
pub struct FrameStats {
    frames: Vec<(Duration, Duration)>,
    first: Instant,
    last: Instant,
}

impl FrameStats {
    /// `Some` when the environment asks for it.
    pub fn from_env() -> Option<FrameStats> {
        std::env::var_os("GLASS_FRAME_STATS").map(|_| {
            let now = Instant::now();
            FrameStats {
                frames: Vec::with_capacity(MAX_BURST),
                first: now,
                last: now,
            }
        })
    }

    /// Records a presented frame, just after its present.
    pub fn record(&mut self, render: Duration, present: Duration) {
        let now = Instant::now();
        if !self.frames.is_empty()
            && (now - self.last > BURST_GAP || self.frames.len() >= MAX_BURST)
        {
            eprintln!("{}", self.summary());
            self.frames.clear();
        }
        if self.frames.is_empty() {
            self.first = now;
        }
        self.frames.push((render, present));
        self.last = now;
    }

    fn summary(&self) -> String {
        let n = self.frames.len();
        let span = (self.last - self.first).as_secs_f64();
        let fps = if n > 1 && span > 0.0 {
            (n - 1) as f64 / span
        } else {
            0.0
        };
        let render = percentiles(self.frames.iter().map(|f| f.0));
        let present = percentiles(self.frames.iter().map(|f| f.1));
        format!(
            "frame-stats: n={n} span={span:.3}s fps={fps:.1} render_ms p50={:.2} p95={:.2} max={:.2} \
             present_ms p50={:.2} p95={:.2} max={:.2}",
            render.0, render.1, render.2, present.0, present.1, present.2
        )
    }
}

fn percentiles(d: impl Iterator<Item = Duration>) -> (f64, f64, f64) {
    let mut v: Vec<f64> = d.map(|x| x.as_secs_f64() * 1000.0).collect();
    v.sort_by(f64::total_cmp);
    let at = |q: usize| v[(v.len() - 1) * q / 100];
    (at(50), at(95), at(100))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_of_a_sorted_range() {
        let d = (1..=101).map(Duration::from_millis);
        let (p50, p95, max) = percentiles(d);
        assert_eq!((p50, p95, max), (51.0, 96.0, 101.0));
    }
}
