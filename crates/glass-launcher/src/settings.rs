//! The settings cards and the card menus.
//!
//! Settings cards sit on the backward side of the timeline, after the "now" cards: Wi-Fi,
//! Bluetooth, screen timeout, head wake, wink, on-head detection, nudge to sleep, device.
//! What they show comes from [`SettingsView`]: the `Settings` that `glassd` applied (so a
//! clamped value is shown as applied), the platform's radios, and the service's
//! `Welcome`. A choice in a menu becomes a [`Choice`]: a `SetSettings` patch, a radio
//! change, the wake-angle sub-screen, or deleting a card.

use crate::platform::RadioStatus;
use glass_api::hub::settings_rules;
use glass_api::proto::Settings;
use glass_ui::{Adjuster, Card, CardContent, CardId, MenuItem};

/// Card ids of the settings cards (fixed, so updates find them).
pub mod ids {
    use glass_ui::CardId;
    pub const WIFI: CardId = 101;
    pub const BLUETOOTH: CardId = 102;
    pub const SCREEN_TIMEOUT: CardId = 103;
    pub const HEAD_WAKE: CardId = 104;
    pub const WINK: CardId = 105;
    pub const ON_HEAD: CardId = 106;
    pub const NUDGE: CardId = 107;
    pub const DEVICE: CardId = 108;
}

/// The settings cards from the one next to the "now" cards to the farthest.
pub const ORDER: [CardId; 8] = [
    ids::WIFI,
    ids::BLUETOOTH,
    ids::SCREEN_TIMEOUT,
    ids::HEAD_WAKE,
    ids::WINK,
    ids::ON_HEAD,
    ids::NUDGE,
    ids::DEVICE,
];

/// Screen timeout choices (glassd allows 3 s to 10 min).
pub const TIMEOUT_CHOICES_MS: [u32; 8] = [
    5_000, 10_000, 15_000, 30_000, 60_000, 120_000, 300_000, 600_000,
];

/// One step of the wake-angle sub-screen, in degrees.
pub const WAKE_ANGLE_STEP: f32 = 5.0;

/// What `glassd` said about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceInfo {
    pub version: String,
    pub capabilities: Vec<String>,
}

/// Everything the settings cards show.
#[derive(Clone, Debug)]
pub struct SettingsView {
    /// The applied settings; `None` until glassd sent them.
    pub settings: Option<Settings>,
    pub wifi: RadioStatus,
    pub bluetooth: RadioStatus,
    /// `None` without glassd.
    pub service: Option<ServiceInfo>,
    pub platform: &'static str,
    pub renderer: &'static str,
}

/// What a menu item does.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    /// Send `SetSettings` with these fields.
    Patch(Settings),
    Wifi(bool),
    Bluetooth(bool),
    /// Open the wake-angle sub-screen.
    WakeAngle,
    /// Remove the card from the timeline.
    Delete(CardId),
}

/// "15 seconds", "1 minute", "10 minutes".
pub fn timeout_text(ms: u32) -> String {
    if ms >= 60_000 && ms % 60_000 == 0 {
        let m = ms / 60_000;
        return if m == 1 {
            "1 minute".into()
        } else {
            format!("{m} minutes")
        };
    }
    let s = ms as f32 / 1000.0;
    if ms % 1000 == 0 {
        format!("{s:.0} seconds")
    } else {
        format!("{s:.1} seconds")
    }
}

/// "30°" (whole degrees; glassd may store a fraction).
pub fn angle_text(deg: f32) -> String {
    format!("{deg:.0}°")
}

fn on_off(v: bool) -> &'static str {
    if v {
        "On"
    } else {
        "Off"
    }
}

fn radio_text(r: &RadioStatus) -> String {
    match (r.enabled, r.connected.as_slice()) {
        (false, _) => "Off".into(),
        (true, []) => "On, not connected".into(),
        (true, [one]) => format!("Connected to {one}"),
        (true, [first, rest @ ..]) => format!("Connected to {first} and {} more", rest.len()),
    }
}

const WAITING: &str = "Waiting for glassd";

fn setting_text(v: &SettingsView, f: impl Fn(&Settings) -> Option<String>) -> String {
    v.settings
        .as_ref()
        .and_then(f)
        .unwrap_or_else(|| WAITING.into())
}

/// What a settings card shows; `None` for an id that is not a settings card.
pub fn card_content(id: CardId, v: &SettingsView) -> Option<CardContent> {
    let entry = |title: &str, subtitle: String| CardContent::Settings {
        title: title.into(),
        subtitle: Some(subtitle),
    };
    Some(match id {
        ids::WIFI => entry("Wi-Fi", radio_text(&v.wifi)),
        ids::BLUETOOTH => entry("Bluetooth", radio_text(&v.bluetooth)),
        ids::SCREEN_TIMEOUT => entry(
            "Screen timeout",
            setting_text(v, |s| s.screen_timeout_ms.map(timeout_text)),
        ),
        ids::HEAD_WAKE => entry(
            "Head wake",
            setting_text(v, |s| {
                s.head_wake_enabled
                    .map(|on| match (on, s.head_wake_angle_deg) {
                        (true, Some(a)) => format!("On, at {}", angle_text(a)),
                        (on, _) => on_off(on).into(),
                    })
            }),
        ),
        ids::WINK => entry(
            "Wink",
            setting_text(v, |s| s.wink_enabled.map(|b| on_off(b).into())),
        ),
        ids::ON_HEAD => entry(
            "On-head detection",
            setting_text(v, |s| s.on_head_detection_enabled.map(|b| on_off(b).into())),
        ),
        ids::NUDGE => entry(
            "Nudge to sleep",
            setting_text(v, |s| s.nudge_sleep_enabled.map(|b| on_off(b).into())),
        ),
        ids::DEVICE => {
            let mut lines = vec![format!(
                "glass-launcher {}, renderer {}",
                env!("CARGO_PKG_VERSION"),
                v.renderer
            )];
            match &v.service {
                Some(s) => {
                    lines.push(format!("glassd {}", s.version));
                    lines.push(s.capabilities.join(", "));
                }
                None => lines.push("glassd not connected".into()),
            }
            lines.push(format!("Platform: {}", v.platform));
            CardContent::Info {
                title: "Device".into(),
                lines,
            }
        }
        _ => return None,
    })
}

/// The settings cards in timeline order (from the "now" cards outward).
pub fn cards(v: &SettingsView) -> Vec<Card> {
    ORDER
        .iter()
        .map(|&id| Card::new(id, card_content(id, v).expect("a settings id")))
        .collect()
}

fn patch(f: impl FnOnce(&mut Settings)) -> Choice {
    let mut s = Settings::default();
    f(&mut s);
    Choice::Patch(s)
}

fn toggle_item(enabled: Option<bool>, set: fn(&mut Settings, bool)) -> MenuItem<Choice> {
    // Unknown: offer to turn it on (the glassd default for most switches is off).
    let on = enabled.unwrap_or(false);
    let label = if on { "Turn off" } else { "Turn on" };
    MenuItem::new(patch(|s| set(s, !on)), label)
}

/// The menu of `card`, with the item to show first; `None` when it has no menu.
pub fn menu_for(card: &Card, v: &SettingsView) -> Option<(Vec<MenuItem<Choice>>, usize)> {
    let s = v.settings.unwrap_or_default();
    let known = v.settings.is_some();
    let items = match card.id {
        ids::WIFI => vec![radio_item(&v.wifi, Choice::Wifi)],
        ids::BLUETOOTH => vec![radio_item(&v.bluetooth, Choice::Bluetooth)],
        ids::SCREEN_TIMEOUT => {
            let items: Vec<MenuItem<Choice>> = TIMEOUT_CHOICES_MS
                .iter()
                .map(|&ms| {
                    let it =
                        MenuItem::new(patch(|p| p.screen_timeout_ms = Some(ms)), timeout_text(ms));
                    if known && s.screen_timeout_ms == Some(ms) {
                        it.with_detail("Current")
                    } else {
                        it
                    }
                })
                .collect();
            let first = s
                .screen_timeout_ms
                .and_then(|ms| TIMEOUT_CHOICES_MS.iter().position(|&c| c == ms))
                .unwrap_or(2);
            return Some((items, first));
        }
        ids::HEAD_WAKE => {
            let angle = s
                .head_wake_angle_deg
                .unwrap_or(settings_defaults().head_wake_angle_deg.unwrap_or(30.0));
            vec![
                toggle_item(s.head_wake_enabled, |p, b| p.head_wake_enabled = Some(b)),
                MenuItem::new(Choice::WakeAngle, "Wake angle").with_detail(angle_text(angle)),
            ]
        }
        ids::WINK => vec![toggle_item(s.wink_enabled, |p, b| p.wink_enabled = Some(b))],
        ids::ON_HEAD => vec![toggle_item(s.on_head_detection_enabled, |p, b| {
            p.on_head_detection_enabled = Some(b)
        })],
        ids::NUDGE => vec![toggle_item(s.nudge_sleep_enabled, |p, b| {
            p.nudge_sleep_enabled = Some(b)
        })],
        ids::DEVICE => return None,
        _ => match card.content {
            CardContent::Text { .. } | CardContent::Image { .. } => {
                vec![MenuItem::new(Choice::Delete(card.id), "Delete")]
            }
            _ => return None,
        },
    };
    Some((items, 0))
}

fn radio_item(r: &RadioStatus, choice: fn(bool) -> Choice) -> MenuItem<Choice> {
    if r.enabled {
        MenuItem::new(choice(false), "Turn off")
    } else {
        MenuItem::new(choice(true), "Turn on")
    }
}

/// The glassd defaults (docs/api.md), used when nothing better is known.
pub fn settings_defaults() -> Settings {
    settings_rules::defaults()
}

/// Applies a patch the way glassd does (clamped), for runs without glassd.
pub fn apply_locally(current: &mut Settings, change: &Settings) {
    settings_rules::merge(current, change);
}

/// The wake-angle sub-screen, starting at the current angle.
pub fn wake_angle_adjuster(v: &SettingsView) -> Adjuster {
    let start = v
        .settings
        .and_then(|s| s.head_wake_angle_deg)
        .unwrap_or(30.0);
    Adjuster::new(
        "Wake angle",
        settings_rules::HEAD_WAKE_ANGLE_MIN,
        settings_rules::HEAD_WAKE_ANGLE_MAX,
        WAKE_ANGLE_STEP,
        start,
        angle_text,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(settings: Option<Settings>) -> SettingsView {
        SettingsView {
            settings,
            wifi: RadioStatus::on(&["HomeNet"]),
            bluetooth: RadioStatus::off(),
            service: None,
            platform: "mock",
            renderer: "software",
        }
    }

    fn subtitle(c: CardContent) -> String {
        match c {
            CardContent::Settings {
                subtitle: Some(s), ..
            } => s,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn texts() {
        assert_eq!(timeout_text(15_000), "15 seconds");
        assert_eq!(timeout_text(3_000), "3 seconds");
        assert_eq!(timeout_text(2_500), "2.5 seconds");
        assert_eq!(timeout_text(60_000), "1 minute");
        assert_eq!(timeout_text(90_000), "90 seconds");
        assert_eq!(timeout_text(600_000), "10 minutes");
        assert_eq!(angle_text(29.6), "30°");
        assert_eq!(
            radio_text(&RadioStatus::on(&["a", "b", "c"])),
            "Connected to a and 2 more"
        );
    }

    #[test]
    fn cards_show_the_applied_settings() {
        let v = view(Some(settings_defaults()));
        assert_eq!(
            subtitle(card_content(ids::SCREEN_TIMEOUT, &v).unwrap()),
            "15 seconds"
        );
        assert_eq!(
            subtitle(card_content(ids::HEAD_WAKE, &v).unwrap()),
            "On, at 30°"
        );
        assert_eq!(subtitle(card_content(ids::WINK, &v).unwrap()), "Off");
        assert_eq!(
            subtitle(card_content(ids::WIFI, &v).unwrap()),
            "Connected to HomeNet"
        );
        assert_eq!(subtitle(card_content(ids::BLUETOOTH, &v).unwrap()), "Off");
        let w = view(None);
        assert_eq!(subtitle(card_content(ids::NUDGE, &w).unwrap()), WAITING);
        assert!(card_content(10, &w).is_none());
        assert_eq!(cards(&w).len(), ORDER.len());
    }

    #[test]
    fn menus_offer_the_opposite_state_and_the_current_timeout() {
        let v = view(Some(settings_defaults()));
        let card = |id| Card::new(id, card_content(id, &v).unwrap());
        let (items, first) = menu_for(&card(ids::HEAD_WAKE), &v).unwrap();
        assert_eq!(first, 0);
        assert_eq!(items[0].label, "Turn off");
        assert_eq!(
            items[0].value,
            Choice::Patch(Settings {
                head_wake_enabled: Some(false),
                ..Default::default()
            })
        );
        assert_eq!(items[1].value, Choice::WakeAngle);
        let (items, first) = menu_for(&card(ids::SCREEN_TIMEOUT), &v).unwrap();
        assert_eq!(items.len(), TIMEOUT_CHOICES_MS.len());
        assert_eq!(items[first].label, "15 seconds");
        assert_eq!(items[first].detail.as_deref(), Some("Current"));
        let (items, _) = menu_for(&card(ids::WINK), &v).unwrap();
        assert_eq!(items[0].label, "Turn on");
        let (items, _) = menu_for(&card(ids::WIFI), &v).unwrap();
        assert_eq!(items[0].value, Choice::Wifi(false));
        let (items, _) = menu_for(&card(ids::BLUETOOTH), &v).unwrap();
        assert_eq!(items[0].value, Choice::Bluetooth(true));
        assert!(menu_for(&card(ids::DEVICE), &v).is_none());
        assert!(menu_for(&Card::new(10, CardContent::Home), &v).is_none());
        let text = Card::new(
            11,
            CardContent::Text {
                body: "x".into(),
                source: "y".into(),
                timestamp_s: 0,
            },
        );
        assert_eq!(menu_for(&text, &v).unwrap().0[0].value, Choice::Delete(11));
    }

    #[test]
    fn local_application_clamps_like_glassd() {
        let mut s = settings_defaults();
        apply_locally(
            &mut s,
            &Settings {
                head_wake_angle_deg: Some(55.0),
                screen_timeout_ms: Some(1_000),
                ..Default::default()
            },
        );
        assert_eq!(s.head_wake_angle_deg, Some(40.0));
        assert_eq!(s.screen_timeout_ms, Some(3_000));
    }

    #[test]
    fn the_wake_angle_screen_starts_at_the_current_angle() {
        let mut s = settings_defaults();
        s.head_wake_angle_deg = Some(20.0);
        assert_eq!(wake_angle_adjuster(&view(Some(s))).value(), 20.0);
        assert_eq!(wake_angle_adjuster(&view(None)).value(), 30.0);
    }
}
