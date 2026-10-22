//! Demo content for the mock launcher until cards come from services.

use crate::platform::{MockPlatform, Platform};
use crate::settings::{self, SettingsView};
use glass_ui::{Card, CardContent, Timeline};

/// A fixed "now" for reproducible demo frames: 2026-10-21 10:42:00 UTC.
pub const DEMO_NOW_S: i64 = 1_792_579_320;

/// A timeline with the settings cards, a "now" card, the home clock and four history cards,
/// with times relative to [`DEMO_NOW_S`].
pub fn timeline() -> Timeline {
    timeline_at(DEMO_NOW_S)
}

/// The demo timeline with times relative to `now_s`.
pub fn timeline_at(now_s: i64) -> Timeline {
    let text = |id, body: &str, source: &str, age_s: i64| {
        Card::new(
            id,
            CardContent::Text {
                body: body.into(),
                source: source.into(),
                timestamp_s: now_s - age_s,
            },
        )
    };
    // The settings cards start with glassd's defaults and the mock radios; the app
    // refreshes them from what it learns.
    let view = SettingsView {
        settings: Some(settings::settings_defaults()),
        wifi: MockPlatform::default().wifi(),
        bluetooth: MockPlatform::default().bluetooth(),
        service: None,
        platform: "mock",
        renderer: "software",
    };
    let mut backward = vec![text(2, "Partly cloudy, 14°", "Weather", 20 * 60)];
    backward.extend(settings::cards(&view));
    Timeline::new(
        backward,
        Card::new(10, CardContent::Home),
        vec![
            text(
                11,
                "Running ten minutes late, save me a seat by the window.",
                "Messages",
                5 * 60,
            ),
            text(12, "Battery fully charged.", "OpenMyGlass", 2 * 3600),
            Card::new(
                13,
                CardContent::Image {
                    image: None,
                    caption: "Photo".into(),
                    timestamp_s: now_s - 26 * 3600,
                },
            ),
            text(
                14,
                "Welcome. Swipe forward for older cards, backward for settings, down to go back.",
                "OpenMyGlass",
                3 * 86_400,
            ),
        ],
    )
}

/// The timeline a device starts with: the settings cards, the home clock and a welcome card
/// (no demo content). Settings show "Waiting for glassd" until its snapshot.
pub fn device_timeline(now_s: i64, platform: &'static str, renderer: &'static str) -> Timeline {
    let view = SettingsView {
        settings: None,
        wifi: crate::platform::RadioStatus::off(),
        bluetooth: crate::platform::RadioStatus::off(),
        service: None,
        platform,
        renderer,
    };
    Timeline::new(
        settings::cards(&view),
        Card::new(10, CardContent::Home),
        vec![Card::new(
            14,
            CardContent::Text {
                body: "Welcome. Swipe forward for older cards, backward for settings, down to go back."
                    .into(),
                source: "OpenMyGlass".into(),
                timestamp_s: now_s,
            },
        )],
    )
}
