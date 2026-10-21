//! The wall clock for the home card.

use glass_ui::RenderContext;
use std::time::{SystemTime, UNIX_EPOCH};

/// Now, with the local hour and minute where the platform tells them (Unix), else UTC.
pub fn now() -> RenderContext {
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (hour, minute) = local_hour_minute(now_s);
    RenderContext {
        now_s,
        hour,
        minute,
    }
}

/// A fixed clock: `now_s` and the given wall-clock time.
pub fn fixed(now_s: i64, hour: u8, minute: u8) -> RenderContext {
    RenderContext {
        now_s,
        hour,
        minute,
    }
}

/// Parses `HH:MM` (24-hour).
pub fn parse_hh_mm(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.split_once(':')?;
    let (h, m) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

#[cfg(unix)]
#[allow(deprecated)] // libc::time_t on musl: 32-bit until the libc crate follows musl 1.2
fn local_hour_minute(now_s: i64) -> (u8, u8) {
    let t = now_s as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which outlives the call.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return utc_hour_minute(now_s);
        }
        (tm.tm_hour as u8, tm.tm_min as u8)
    }
}

#[cfg(not(unix))]
fn local_hour_minute(now_s: i64) -> (u8, u8) {
    utc_hour_minute(now_s)
}

fn utc_hour_minute(now_s: i64) -> (u8, u8) {
    let day_s = now_s.rem_euclid(86_400);
    ((day_s / 3_600) as u8, (day_s % 3_600 / 60) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_times() {
        assert_eq!(parse_hh_mm("10:42"), Some((10, 42)));
        assert_eq!(parse_hh_mm("0:05"), Some((0, 5)));
        assert_eq!(parse_hh_mm("24:00"), None);
        assert_eq!(parse_hh_mm("10:60"), None);
        assert_eq!(parse_hh_mm("1042"), None);
    }

    #[test]
    fn utc_fallback() {
        assert_eq!(utc_hour_minute(super::super::demo::DEMO_NOW_S), (10, 42));
    }
}
