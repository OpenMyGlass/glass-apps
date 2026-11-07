//! Proof of life for the supervisor of the GPU launcher (`--heartbeat PATH`): a counter
//! rewritten in a file on tmpfs from the main loop, so a hang in the driver stops it. Timers
//! stop in deep sleep for this process and for the supervisor alike, so sleep is no silence.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long the main loop may block while it has a heartbeat: the supervisor gives up after
/// 20 s of silence, and this wake costs nothing in deep sleep.
pub const PERIOD: Duration = Duration::from_secs(5);

/// A frame burst must not write the file at every frame.
const MIN_GAP: Duration = Duration::from_secs(1);

pub struct Heartbeat {
    path: PathBuf,
    count: u64,
    last: Option<Instant>,
    failed: bool,
}

impl Heartbeat {
    pub fn new(path: PathBuf) -> Heartbeat {
        Heartbeat {
            path,
            count: 0,
            last: None,
            failed: false,
        }
    }

    /// Writes the next count, unless the last write is under a second old.
    pub fn beat(&mut self) {
        if self.last.is_some_and(|t| t.elapsed() < MIN_GAP) {
            return;
        }
        self.last = Some(Instant::now());
        self.count += 1;
        match std::fs::write(&self.path, self.count.to_string()) {
            Ok(()) => self.failed = false,
            Err(e) if !self.failed => {
                // Once: the supervisor will see the silence and act on it.
                self.failed = true;
                eprintln!("glass-launcher: heartbeat {}: {e}", self.path.display());
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_at_most_once_a_second() {
        let dir = std::env::temp_dir().join(format!("glass-heartbeat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("beat");
        let mut h = Heartbeat::new(path.clone());
        h.beat();
        h.beat();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1");
        h.last = Some(Instant::now() - MIN_GAP);
        h.beat();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "2");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failing_write_does_not_stop_the_count() {
        let mut h = Heartbeat::new(PathBuf::from("/nonexistent-dir/beat"));
        h.beat();
        assert!(h.failed);
        assert_eq!(h.count, 1);
    }
}
