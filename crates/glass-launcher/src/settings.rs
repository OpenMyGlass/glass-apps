//! The settings cards and the card menus.
//!
//! Settings cards sit on the backward side of the timeline, after the "now" cards: Wi-Fi,
//! Bluetooth, the phone (pairing, with glassd's `companion-link`; see [`crate::pairing`]),
//! screen timeout, head wake, on-head detection, power off when removed, nudge to
//! sleep, device (wink is hidden until the glasshub firmware delivers wink events).
//! What they show comes from [`SettingsView`]: the `Settings` that `glassd` applied (so a
//! clamped value is shown as applied), the platform's radios, and the service's
//! `Welcome`. A choice in a menu becomes a [`Choice`]: a `SetSettings` patch, a radio
//! change, the wake-angle sub-screen, or deleting a card.

use crate::platform::RadioStatus;
use glass_api::hub::settings_rules;
use glass_api::proto::Settings;
use glass_ui::i18n::{fmt1, fmt2, Locale, Msg};
use glass_ui::{Adjuster, Card, CardContent, CardId, MenuItem, Timeline};

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
    pub const LANGUAGE: CardId = 108;
    pub const DEVICE: CardId = 109;
    /// The one card on the main timeline; a tap opens the settings cards.
    pub const ROOT: CardId = 110;
    pub const VOLUME: CardId = 111;
    pub const TIME_FORMAT: CardId = 112;
    pub const TIMEZONE: CardId = 113;
    pub const POWER_OFF: CardId = 114;
    /// "Pair phone", or the paired phone (only with glassd's `companion-link` capability).
    pub const PHONE: CardId = 115;
    /// The menus that ask before forgetting the phone and after a phone bonded. They belong
    /// to no card of [`super::ORDER`], so a settings change does not rebuild them.
    pub const PHONE_QUESTION: CardId = 116;
    /// The question asked after any bond ([`super::paired_question`]); while it is unanswered
    /// it comes back at each wake.
    pub const BOND_QUESTION: CardId = 117;
}

/// The settings cards from the one next to the "now" cards to the farthest.
///
/// The wink card ([`ids::WINK`]) is not shown: the Glass glasshub firmware (1.22) never
/// delivers a wink event, so the toggle did nothing the owner could see. `glassd` keeps the
/// setting; the card comes back here once the MCU wink threshold is tuned.
pub const ORDER: [CardId; 13] = [
    ids::WIFI,
    ids::BLUETOOTH,
    ids::PHONE,
    ids::VOLUME,
    ids::SCREEN_TIMEOUT,
    ids::HEAD_WAKE,
    ids::ON_HEAD,
    ids::POWER_OFF,
    ids::NUDGE,
    ids::TIME_FORMAT,
    ids::TIMEZONE,
    ids::LANGUAGE,
    ids::DEVICE,
];

/// Screen timeout choices (glassd allows 3 s to 10 min).
pub const TIMEOUT_CHOICES_MS: [u32; 8] = [
    5_000, 10_000, 15_000, 30_000, 60_000, 120_000, 300_000, 600_000,
];

/// Minutes off the head before powering off (glassd allows 0, meaning never, to 12 hours).
pub const POWER_OFF_CHOICES_MIN: [u32; 5] = [10, 30, 60, 120, 0];

/// One step of the wake-angle sub-screen, in degrees.
pub const WAKE_ANGLE_STEP: f32 = 5.0;

/// What `glassd` said about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceInfo {
    pub version: String,
    pub capabilities: Vec<String>,
}

/// The phone bond, as glassd reports it, for the "Pair phone" card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhoneView {
    pub paired: bool,
    pub paired_name: String,
    /// glassd answered `ERROR_CODE_UNSUPPORTED` to a pairing request: it has no pairing mode.
    pub unsupported: bool,
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
    /// The language every card text is built in.
    pub locale: Locale,
    pub phone: PhoneView,
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
    /// Change the UI language (persisted through glassd's settings).
    SetLocale(Locale),
    /// Remove the card from the timeline.
    Delete(CardId),
    /// Open the pairing mode; first, with `radios`, turn Bluetooth on and airplane mode off
    /// with this patch (the menu item said so).
    Pair {
        radios: Option<Settings>,
    },
    /// Ask before forgetting the phone.
    AskForget,
    /// Forget the bonded phone (`CompanionForgetBond`).
    Forget,
    /// The owner says the phone that bonded is theirs (nothing to send).
    KeepPhone,
    /// Ask about the phone that bonded again (its name was tapped, not an answer).
    AskBond,
    /// Close the menu, nothing else.
    Dismiss,
}

/// "15 seconds", "1 minute", "10 minutes" (in `locale`).
pub fn timeout_text(locale: Locale, ms: u32) -> String {
    if ms >= 60_000 && ms % 60_000 == 0 {
        let m = ms / 60_000;
        return if m == 1 {
            Msg::MinuteOne.get(locale).to_string()
        } else {
            fmt1(Msg::MinutesN.get(locale), &m.to_string())
        };
    }
    let s = ms as f32 / 1000.0;
    // The number keeps ASCII digits (a decimal only for a value off the fixed list); the
    // unit word is localised.
    let n = if ms % 1000 == 0 {
        format!("{s:.0}")
    } else {
        format!("{s:.1}")
    };
    fmt1(Msg::SecondsN.get(locale), &n)
}

/// "30 minutes", or "Never" for 0 (in `locale`).
pub fn power_off_text(locale: Locale, minutes: u32) -> String {
    match minutes {
        0 => Msg::Never.get(locale).to_string(),
        m => timeout_text(locale, m.saturating_mul(60_000)),
    }
}

/// "30°" (whole degrees; glassd may store a fraction). Language-neutral.
pub fn angle_text(deg: f32) -> String {
    format!("{deg:.0}°")
}

fn on_off(locale: Locale, v: bool) -> &'static str {
    if v {
        Msg::On.get(locale)
    } else {
        Msg::Off.get(locale)
    }
}

fn radio_text(locale: Locale, r: &RadioStatus) -> String {
    match (r.enabled, r.connected.as_slice()) {
        (false, _) => Msg::Off.get(locale).to_string(),
        (true, []) => Msg::RadioOnNotConnected.get(locale).to_string(),
        (true, [one]) => fmt1(Msg::ConnectedTo.get(locale), one),
        (true, [first, rest @ ..]) => fmt2(
            Msg::ConnectedToMore.get(locale),
            first,
            &rest.len().to_string(),
        ),
    }
}

fn setting_text(v: &SettingsView, f: impl Fn(&Settings) -> Option<String>) -> String {
    v.settings
        .as_ref()
        .and_then(f)
        .unwrap_or_else(|| Msg::WaitingGlassd.get(v.locale).to_string())
}

/// What a settings card shows; `None` for an id that is not a settings card.
pub fn card_content(id: CardId, v: &SettingsView) -> Option<CardContent> {
    let loc = v.locale;
    let entry = |title: Msg, subtitle: String| CardContent::Settings {
        title: title.get(loc).into(),
        subtitle: Some(subtitle),
    };
    Some(match id {
        ids::WIFI => entry(Msg::SettingsWifi, radio_text(loc, &v.wifi)),
        ids::BLUETOOTH => entry(Msg::SettingsBluetooth, radio_text(loc, &v.bluetooth)),
        ids::SCREEN_TIMEOUT => entry(
            Msg::SettingsScreenTimeout,
            setting_text(v, |s| s.screen_timeout_ms.map(|ms| timeout_text(loc, ms))),
        ),
        ids::HEAD_WAKE => entry(
            Msg::SettingsHeadWake,
            setting_text(v, |s| {
                s.head_wake_enabled
                    .map(|on| match (on, s.head_wake_angle_deg) {
                        (true, Some(a)) => fmt1(Msg::HeadWakeOnAt.get(loc), &angle_text(a)),
                        (on, _) => on_off(loc, on).into(),
                    })
            }),
        ),
        ids::WINK => entry(
            Msg::SettingsWink,
            setting_text(v, |s| s.wink_enabled.map(|b| on_off(loc, b).into())),
        ),
        ids::ON_HEAD => entry(
            Msg::SettingsOnHead,
            setting_text(v, |s| {
                s.on_head_detection_enabled.map(|b| on_off(loc, b).into())
            }),
        ),
        ids::POWER_OFF => entry(
            Msg::SettingsPowerOff,
            setting_text(v, |s| {
                s.off_head_power_off_min.map(|m| power_off_text(loc, m))
            }),
        ),
        ids::NUDGE => entry(
            Msg::SettingsNudge,
            setting_text(v, |s| s.nudge_sleep_enabled.map(|b| on_off(loc, b).into())),
        ),
        ids::VOLUME => entry(
            Msg::SettingsVolume,
            setting_text(v, |s| {
                Some(volume_text(
                    s.audio_volume
                        .unwrap_or(settings_rules::AUDIO_VOLUME_DEFAULT) as f32,
                ))
            }),
        ),
        ids::TIME_FORMAT => entry(
            Msg::SettingsTimeFormat,
            setting_text(v, |s| {
                s.time_24h.map(|h24| {
                    if h24 { Msg::Time24h } else { Msg::Time12h }
                        .get(loc)
                        .to_string()
                })
            }),
        ),
        ids::TIMEZONE => entry(
            Msg::SettingsTimezone,
            setting_text(v, |s| {
                s.timezone_id
                    .and_then(|i| settings_rules::TIME_ZONES.get(i as usize))
                    .map(|z| z.0.to_string())
            }),
        ),
        ids::LANGUAGE => entry(Msg::SettingsLanguage, loc.endonym().into()),
        ids::PHONE => {
            let p = &v.phone;
            if p.paired {
                entry(
                    Msg::SettingsPhone,
                    crate::pairing::paired_text(loc, &p.paired_name),
                )
            } else if p.unsupported {
                entry(
                    Msg::SettingsPairPhone,
                    Msg::PairingNotAvailable.get(loc).into(),
                )
            } else {
                entry(Msg::SettingsPairPhone, Msg::TapToPair.get(loc).into())
            }
        }
        ids::DEVICE => {
            let mut lines = vec![fmt2(
                Msg::DeviceLauncherLine.get(loc),
                env!("CARGO_PKG_VERSION"),
                v.renderer,
            )];
            match &v.service {
                Some(s) => {
                    lines.push(fmt1(Msg::DeviceGlassdLine.get(loc), &s.version));
                    lines.push(s.capabilities.join(", "));
                }
                None => lines.push(Msg::DeviceGlassdNotConnected.get(loc).into()),
            }
            lines.push(fmt1(Msg::DevicePlatform.get(loc), v.platform));
            CardContent::Info {
                title: Msg::SettingsDevice.get(loc).into(),
                lines,
            }
        }
        _ => return None,
    })
}

/// The settings cards in timeline order (from the "now" cards outward).
/// The card that stands for the settings on the main timeline.
pub fn root_card(locale: Locale) -> Card {
    Card::new(
        ids::ROOT,
        CardContent::settings(Msg::SettingsRoot.get(locale)),
    )
}

/// The timeline shown when the settings are open: their cards, the first one in front.
pub fn timeline(v: &SettingsView) -> Timeline {
    let mut all = cards(v).into_iter();
    let first = all.next().expect("there are settings cards");
    Timeline::new(vec![], first, all.collect())
}

pub fn cards(v: &SettingsView) -> Vec<Card> {
    ORDER
        .iter()
        .filter(|&&id| applies(id, v))
        .map(|&id| Card::new(id, card_content(id, v).expect("a settings id")))
        .collect()
}

/// Whether the setting does anything here. Only a glassd with the `power` capability powers
/// off (not on Android, where the framework owns power), so a glassd without it gets no card.
/// Only a glassd that holds the phone link itself (`companion-link`, Model B) has a pairing
/// mode, so without glassd or without it there is no phone card (`glassd --mock` has none).
pub fn applies(id: CardId, v: &SettingsView) -> bool {
    let has = |cap: &str| {
        v.service
            .as_ref()
            .map(|s| s.capabilities.iter().any(|c| c == cap))
    };
    match id {
        ids::POWER_OFF => has("power").unwrap_or(true),
        ids::PHONE => has("companion-link").unwrap_or(false),
        _ => true,
    }
}

fn patch(f: impl FnOnce(&mut Settings)) -> Choice {
    let mut s = Settings::default();
    f(&mut s);
    Choice::Patch(s)
}

fn toggle_item(
    loc: Locale,
    enabled: Option<bool>,
    set: fn(&mut Settings, bool),
) -> MenuItem<Choice> {
    // Called only with known settings; a field glassd left out counts as off.
    let on = enabled.unwrap_or(false);
    let label = if on { Msg::TurnOff } else { Msg::TurnOn };
    MenuItem::new(patch(|s| set(s, !on)), label.get(loc))
}

/// The menu of `card`, with the item to show first; `None` when it has no menu.
pub fn menu_for(card: &Card, v: &SettingsView) -> Option<(Vec<MenuItem<Choice>>, usize)> {
    let loc = v.locale;
    let owned_by_glassd = matches!(
        card.id,
        ids::SCREEN_TIMEOUT
            | ids::HEAD_WAKE
            | ids::WINK
            | ids::ON_HEAD
            | ids::POWER_OFF
            | ids::NUDGE
            | ids::TIME_FORMAT
            | ids::TIMEZONE
            | ids::PHONE
    );
    if owned_by_glassd && v.settings.is_none() {
        // Unknown until glassd sends them: no menu, so nothing is written blind.
        return None;
    }
    let s = v.settings.unwrap_or_default();
    let items = match card.id {
        ids::WIFI => vec![radio_item(loc, &v.wifi, Choice::Wifi)],
        ids::BLUETOOTH => vec![radio_item(loc, &v.bluetooth, Choice::Bluetooth)],
        ids::SCREEN_TIMEOUT => {
            // The fixed choices, plus the current value when it is none of them (set by
            // another client), in order.
            let mut choices = TIMEOUT_CHOICES_MS.to_vec();
            if let Some(ms) = s.screen_timeout_ms {
                if !choices.contains(&ms) {
                    choices.push(ms);
                    choices.sort_unstable();
                }
            }
            let items: Vec<MenuItem<Choice>> = choices
                .iter()
                .map(|&ms| {
                    let it = MenuItem::new(
                        patch(|p| p.screen_timeout_ms = Some(ms)),
                        timeout_text(loc, ms),
                    );
                    if s.screen_timeout_ms == Some(ms) {
                        it.with_detail(Msg::Current.get(loc))
                    } else {
                        it
                    }
                })
                .collect();
            let first = s
                .screen_timeout_ms
                .and_then(|ms| choices.iter().position(|&c| c == ms))
                .unwrap_or(2);
            return Some((items, first));
        }
        ids::HEAD_WAKE => {
            let angle = s
                .head_wake_angle_deg
                .unwrap_or(settings_defaults().head_wake_angle_deg.unwrap_or(30.0));
            vec![
                toggle_item(loc, s.head_wake_enabled, |p, b| {
                    p.head_wake_enabled = Some(b)
                }),
                MenuItem::new(Choice::WakeAngle, Msg::WakeAngle.get(loc))
                    .with_detail(angle_text(angle)),
            ]
        }
        ids::WINK => vec![toggle_item(loc, s.wink_enabled, |p, b| {
            p.wink_enabled = Some(b)
        })],
        ids::ON_HEAD => vec![toggle_item(loc, s.on_head_detection_enabled, |p, b| {
            p.on_head_detection_enabled = Some(b)
        })],
        ids::POWER_OFF => {
            // As for the screen timeout, with "never" kept last.
            let mut choices = POWER_OFF_CHOICES_MIN.to_vec();
            if let Some(m) = s.off_head_power_off_min {
                if !choices.contains(&m) {
                    choices.push(m);
                    choices.sort_unstable_by_key(|&c| if c == 0 { u32::MAX } else { c });
                }
            }
            let items: Vec<MenuItem<Choice>> = choices
                .iter()
                .map(|&m| {
                    let it = MenuItem::new(
                        patch(|p| p.off_head_power_off_min = Some(m)),
                        power_off_text(loc, m),
                    );
                    if s.off_head_power_off_min == Some(m) {
                        it.with_detail(Msg::Current.get(loc))
                    } else {
                        it
                    }
                })
                .collect();
            let first = s
                .off_head_power_off_min
                .and_then(|m| choices.iter().position(|&c| c == m))
                .unwrap_or(1);
            return Some((items, first));
        }
        // The toggle explains the gesture: the card title alone ("nudge") was not clear.
        ids::NUDGE => vec![toggle_item(loc, s.nudge_sleep_enabled, |p, b| {
            p.nudge_sleep_enabled = Some(b)
        })
        .with_detail(Msg::NudgeHint.get(loc))],
        ids::TIME_FORMAT => {
            let current = usize::from(!s.time_24h.unwrap_or(true));
            let items = [(true, Msg::Time24h), (false, Msg::Time12h)]
                .into_iter()
                .enumerate()
                .map(|(i, (h24, label))| {
                    let it = MenuItem::new(patch(|p| p.time_24h = Some(h24)), label.get(loc));
                    if i == current {
                        it.with_detail(Msg::Current.get(loc))
                    } else {
                        it
                    }
                })
                .collect();
            return Some((items, current));
        }
        ids::TIMEZONE => {
            let current = s.timezone_id.unwrap_or(settings_rules::TIMEZONE_DEFAULT) as usize;
            let items = settings_rules::TIME_ZONES
                .iter()
                .enumerate()
                .map(|(i, z)| {
                    let it = MenuItem::new(patch(|p| p.timezone_id = Some(i as u32)), z.0);
                    if i == current {
                        it.with_detail(Msg::Current.get(loc))
                    } else {
                        it
                    }
                })
                .collect();
            return Some((items, current));
        }
        ids::LANGUAGE => {
            // One item per language, in the picker's order; the active one shows "Current".
            let items: Vec<MenuItem<Choice>> = Locale::ALL
                .into_iter()
                .map(|l| {
                    let it = MenuItem::new(Choice::SetLocale(l), l.endonym());
                    if l == loc {
                        it.with_detail(Msg::Current.get(loc))
                    } else {
                        it
                    }
                })
                .collect();
            let first = Locale::ALL.iter().position(|&l| l == loc).unwrap_or(0);
            return Some((items, first));
        }
        ids::DEVICE => return None,
        ids::PHONE => return phone_menu(v).map(|items| (items, 0)),
        _ => match card.content {
            CardContent::Text { .. } | CardContent::Image { .. } => {
                vec![MenuItem::new(Choice::Delete(card.id), Msg::Delete.get(loc))]
            }
            _ => return None,
        },
    };
    Some((items, 0))
}

/// The phone card's menu: start pairing (saying first what changes in the radios), or forget
/// the paired phone. `None` while glassd has no pairing mode.
fn phone_menu(v: &SettingsView) -> Option<Vec<MenuItem<Choice>>> {
    let loc = v.locale;
    let p = &v.phone;
    if p.paired {
        let mut it = MenuItem::new(Choice::AskForget, Msg::ForgetPhone.get(loc));
        if !p.paired_name.is_empty() {
            it = it.with_detail(p.paired_name.clone());
        }
        return Some(vec![it]);
    }
    if p.unsupported {
        return None;
    }
    let s = v.settings?;
    let bluetooth_off = !s.bluetooth_enabled.unwrap_or(false);
    let airplane = s.airplane_mode.unwrap_or(false);
    let radios = Settings {
        bluetooth_enabled: bluetooth_off.then_some(true),
        airplane_mode: airplane.then_some(false),
        ..Default::default()
    };
    let item = if airplane {
        let it = MenuItem::new(
            Choice::Pair {
                radios: Some(radios),
            },
            Msg::AirplaneOffAndPair.get(loc),
        );
        if bluetooth_off {
            it.with_detail(Msg::BluetoothTurnsOn.get(loc))
        } else {
            it
        }
    } else if bluetooth_off {
        MenuItem::new(
            Choice::Pair {
                radios: Some(radios),
            },
            Msg::BluetoothOnAndPair.get(loc),
        )
    } else {
        MenuItem::new(Choice::Pair { radios: None }, Msg::StartPairing.get(loc))
            .with_detail(crate::pairing::window_text(loc))
    };
    Some(vec![item])
}

/// Asked before forgetting: cancel comes first, so a second tap by mistake forgets nothing.
pub fn forget_question(loc: Locale) -> Vec<MenuItem<Choice>> {
    vec![
        MenuItem::new(Choice::Dismiss, Msg::Cancel.get(loc)),
        MenuItem::new(Choice::Forget, Msg::ForgetYes.get(loc))
            .with_detail(Msg::ForgetCutsOff.get(loc)),
    ]
}

/// Asked whenever a phone has bonded: the first one in range wins and no code is compared,
/// so the owner checks the name. The menu opens on the name, which answers nothing (a tap
/// asks again): a reflex tap neither keeps a stranger nor forgets the owner's phone, and
/// either answer takes a swipe and a tap. "This is my phone" comes next, as the usual answer.
pub fn paired_question(loc: Locale, name: &str) -> Vec<MenuItem<Choice>> {
    vec![
        MenuItem::new(Choice::AskBond, crate::pairing::paired_text(loc, name))
            .with_detail(Msg::SwipeToAnswer.get(loc)),
        MenuItem::new(Choice::KeepPhone, Msg::ThisIsMyPhone.get(loc)),
        MenuItem::new(Choice::Forget, Msg::NotMyPhone.get(loc)).with_detail(Msg::ForgetIt.get(loc)),
    ]
}

fn radio_item(loc: Locale, r: &RadioStatus, choice: fn(bool) -> Choice) -> MenuItem<Choice> {
    if r.enabled {
        MenuItem::new(choice(false), Msg::TurnOff.get(loc))
    } else {
        MenuItem::new(choice(true), Msg::TurnOn.get(loc))
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

/// A volume percentage.
fn volume_text(percent: f32) -> String {
    format!("{}%", percent.round() as u32)
}

/// The volume sub-screen, starting at the current volume.
pub fn volume_adjuster(v: &SettingsView) -> Adjuster {
    let start = v
        .settings
        .and_then(|s| s.audio_volume)
        .unwrap_or(settings_rules::AUDIO_VOLUME_DEFAULT);
    Adjuster::new(
        Msg::SettingsVolume.get(v.locale),
        0.0,
        settings_rules::AUDIO_VOLUME_MAX as f32,
        1.0,
        start as f32,
        volume_text,
    )
}

/// The wake-angle sub-screen, starting at the current angle.
pub fn wake_angle_adjuster(v: &SettingsView) -> Adjuster {
    let start = v
        .settings
        .and_then(|s| s.head_wake_angle_deg)
        .unwrap_or(30.0);
    Adjuster::new(
        Msg::WakeAngle.get(v.locale),
        settings_rules::HEAD_WAKE_ANGLE_MIN,
        settings_rules::HEAD_WAKE_ANGLE_MAX,
        WAKE_ANGLE_STEP,
        start,
        angle_text,
    )
}

/// The locale glassd carries in its settings, if any — the launcher's single read point for
/// the locale glassd and the companion push (see [`crate::App::settings_changed`]).
///
/// The persisted locale lives in glassd's settings (no separate store); it is absent until
/// someone chooses, which is what sends a first boot to the language picker.
pub fn locale_from_settings(s: &Settings) -> Option<Locale> {
    use glass_api::proto::Locale as P;
    match P::try_from(s.locale?).ok()? {
        P::It => Some(Locale::It),
        P::En => Some(Locale::En),
        P::Fr => Some(Locale::Fr),
        P::Es => Some(Locale::Es),
        P::Unspecified => None,
    }
}

/// A settings change that sets the language.
pub fn locale_settings_patch(locale: Locale) -> Settings {
    Settings {
        locale: Some(locale_to_settings(locale)),
        ..Default::default()
    }
}

/// The `Settings.locale` value for `locale`.
pub fn locale_to_settings(locale: Locale) -> i32 {
    use glass_api::proto::Locale as P;
    (match locale {
        Locale::It => P::It,
        Locale::En => P::En,
        Locale::Fr => P::Fr,
        Locale::Es => P::Es,
    }) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(settings: Option<Settings>) -> SettingsView {
        view_in(settings, Locale::En)
    }

    fn view_in(settings: Option<Settings>, locale: Locale) -> SettingsView {
        SettingsView {
            settings,
            wifi: RadioStatus::on(&["HomeNet"]),
            bluetooth: RadioStatus::off(),
            service: None,
            platform: "mock",
            renderer: "software",
            locale,
            phone: PhoneView::default(),
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
        use Locale::{En, It};
        assert_eq!(timeout_text(En, 15_000), "15 seconds");
        assert_eq!(timeout_text(En, 3_000), "3 seconds");
        assert_eq!(timeout_text(En, 2_500), "2.5 seconds");
        assert_eq!(timeout_text(En, 60_000), "1 minute");
        assert_eq!(timeout_text(En, 90_000), "90 seconds");
        assert_eq!(timeout_text(En, 600_000), "10 minutes");
        assert_eq!(angle_text(29.6), "30°");
        assert_eq!(
            radio_text(En, &RadioStatus::on(&["a", "b", "c"])),
            "Connected to a and 2 more"
        );
        // The same durations and radio status in Italian.
        assert_eq!(timeout_text(It, 15_000), "15 secondi");
        assert_eq!(timeout_text(It, 60_000), "1 minuto");
        assert_eq!(timeout_text(It, 600_000), "10 minuti");
        assert_eq!(
            radio_text(It, &RadioStatus::on(&["a", "b", "c"])),
            "Connesso a a e altri 2"
        );
    }

    #[test]
    fn cards_show_the_applied_settings() {
        let v = view(Some(settings_defaults()));
        assert_eq!(
            subtitle(card_content(ids::SCREEN_TIMEOUT, &v).unwrap()),
            "2 minutes"
        );
        assert_eq!(subtitle(card_content(ids::HEAD_WAKE, &v).unwrap()), "Off");
        assert_eq!(subtitle(card_content(ids::WINK, &v).unwrap()), "Off");
        assert_eq!(
            subtitle(card_content(ids::WIFI, &v).unwrap()),
            "Connected to HomeNet"
        );
        assert_eq!(subtitle(card_content(ids::BLUETOOTH, &v).unwrap()), "Off");
        let w = view(None);
        assert_eq!(
            subtitle(card_content(ids::NUDGE, &w).unwrap()),
            Msg::WaitingGlassd.get(Locale::En)
        );
        assert!(card_content(10, &w).is_none());
        // Without glassd there is no phone card: only its own link has a pairing mode.
        assert_eq!(cards(&w).len(), ORDER.len() - 1);
        assert!(cards(&w).iter().all(|c| c.id != ids::PHONE));
        // The language card shows the current language's own name.
        assert_eq!(
            subtitle(card_content(ids::LANGUAGE, &view_in(None, Locale::Fr)).unwrap()),
            "Français"
        );
    }

    #[test]
    fn cards_follow_the_locale() {
        let v = view_in(Some(settings_defaults()), Locale::It);
        assert_eq!(
            subtitle(card_content(ids::SCREEN_TIMEOUT, &v).unwrap()),
            "2 minuti"
        );
        assert_eq!(subtitle(card_content(ids::WINK, &v).unwrap()), "Disattivo");
        // The language menu offers every language and marks the current one.
        let card = Card::new(ids::LANGUAGE, card_content(ids::LANGUAGE, &v).unwrap());
        let (items, first) = menu_for(&card, &v).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(items[first].label, "Italiano");
        assert_eq!(items[first].value, Choice::SetLocale(Locale::It));
        assert_eq!(items[first].detail.as_deref(), Some("Attuale"));
    }

    #[test]
    fn menus_offer_the_opposite_state_and_the_current_timeout() {
        let v = view(Some(settings_defaults()));
        let card = |id| Card::new(id, card_content(id, &v).unwrap());
        let (items, first) = menu_for(&card(ids::HEAD_WAKE), &v).unwrap();
        assert_eq!(first, 0);
        assert_eq!(items[0].label, "Turn on");
        assert_eq!(
            items[0].value,
            Choice::Patch(Settings {
                head_wake_enabled: Some(true),
                ..Default::default()
            })
        );
        assert_eq!(items[1].value, Choice::WakeAngle);
        let (items, first) = menu_for(&card(ids::SCREEN_TIMEOUT), &v).unwrap();
        assert_eq!(items.len(), TIMEOUT_CHOICES_MS.len());
        assert_eq!(items[first].label, "2 minutes");
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
    fn no_menu_for_glassd_settings_until_they_are_known() {
        let v = view(None);
        for id in [
            ids::SCREEN_TIMEOUT,
            ids::HEAD_WAKE,
            ids::WINK,
            ids::ON_HEAD,
            ids::POWER_OFF,
            ids::NUDGE,
        ] {
            let card = Card::new(id, card_content(id, &v).unwrap());
            assert!(menu_for(&card, &v).is_none(), "card {id}");
        }
        // The radios belong to the platform: their menus do not wait for glassd.
        let wifi = Card::new(ids::WIFI, card_content(ids::WIFI, &v).unwrap());
        assert!(menu_for(&wifi, &v).is_some());
    }

    #[test]
    fn a_timeout_outside_the_choices_is_offered_as_current() {
        let mut s = settings_defaults();
        s.screen_timeout_ms = Some(20_000);
        let v = view(Some(s));
        let card = Card::new(
            ids::SCREEN_TIMEOUT,
            card_content(ids::SCREEN_TIMEOUT, &v).unwrap(),
        );
        let (items, first) = menu_for(&card, &v).unwrap();
        assert_eq!(items.len(), TIMEOUT_CHOICES_MS.len() + 1);
        assert_eq!(first, 3, "between 15 and 30 seconds");
        assert_eq!(items[first].label, "20 seconds");
        assert_eq!(items[first].detail.as_deref(), Some("Current"));
        assert_eq!(items.iter().filter(|i| i.detail.is_some()).count(), 1);
    }

    #[test]
    fn the_power_off_menu_offers_the_choices_and_never_last() {
        let v = view(Some(settings_defaults()));
        let card = Card::new(ids::POWER_OFF, card_content(ids::POWER_OFF, &v).unwrap());
        assert_eq!(subtitle(card.content.clone()), "30 minutes");
        let (items, first) = menu_for(&card, &v).unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "10 minutes",
                "30 minutes",
                "60 minutes",
                "120 minutes",
                "Never"
            ]
        );
        assert_eq!(items[first].label, "30 minutes");
        assert_eq!(items[first].detail.as_deref(), Some("Current"));
        assert_eq!(
            items[4].value,
            Choice::Patch(Settings {
                off_head_power_off_min: Some(0),
                ..Default::default()
            })
        );

        // A value set by another client is offered in order, before "never".
        let mut s = settings_defaults();
        s.off_head_power_off_min = Some(45);
        let v = view(Some(s));
        let (items, first) = menu_for(&card, &v).unwrap();
        assert_eq!(first, 2);
        assert_eq!(items[first].label, "45 minutes");
        assert_eq!(items.last().unwrap().label, "Never");

        s.off_head_power_off_min = Some(0);
        let v = view_in(Some(s), Locale::It);
        assert_eq!(subtitle(card_content(ids::POWER_OFF, &v).unwrap()), "Mai");
        let (items, first) = menu_for(&card, &v).unwrap();
        assert_eq!(items[first].label, "Mai");
        assert_eq!(items[first].detail.as_deref(), Some("Attuale"));
    }

    #[test]
    fn the_power_off_card_needs_a_glassd_that_powers_off() {
        let shown = |capabilities: Option<&[&str]>| {
            let mut v = view(Some(settings_defaults()));
            v.service = capabilities.map(|c| ServiceInfo {
                version: "x".into(),
                capabilities: c.iter().map(|s| s.to_string()).collect(),
            });
            cards(&v).iter().any(|c| c.id == ids::POWER_OFF)
        };
        assert!(shown(Some(&["screen", "power"])));
        assert!(!shown(Some(&["screen"])), "Android: no power capability");
        assert!(shown(None), "without glassd the change is only local");
    }

    fn phone_view(s: Settings, phone: PhoneView) -> SettingsView {
        let mut v = view(Some(s));
        v.service = Some(ServiceInfo {
            version: "x".into(),
            capabilities: vec!["companion-link".into()],
        });
        v.phone = phone;
        v
    }

    fn phone_menu_of(v: &SettingsView) -> Option<Vec<MenuItem<Choice>>> {
        let card = Card::new(ids::PHONE, card_content(ids::PHONE, v).unwrap());
        menu_for(&card, v).map(|(items, _)| items)
    }

    #[test]
    fn the_phone_card_needs_a_glassd_that_holds_the_link() {
        let shown = |capabilities: Option<&[&str]>| {
            let mut v = view(Some(settings_defaults()));
            v.service = capabilities.map(|c| ServiceInfo {
                version: "x".into(),
                capabilities: c.iter().map(|s| s.to_string()).collect(),
            });
            cards(&v).iter().any(|c| c.id == ids::PHONE)
        };
        assert!(shown(Some(&["screen", "companion-link"])));
        assert!(!shown(Some(&["mock", "screen"])), "glassd --mock, Android");
        assert!(!shown(None), "without glassd nothing pairs");
        // Right after Bluetooth.
        assert_eq!(ORDER[2], ids::PHONE);
    }

    #[test]
    fn pairing_asks_before_changing_the_radios() {
        let mut s = settings_defaults();
        // Defaults: Bluetooth off, airplane mode off.
        let v = phone_view(s, PhoneView::default());
        assert_eq!(
            subtitle(card_content(ids::PHONE, &v).unwrap()),
            "Tap to pair"
        );
        let items = phone_menu_of(&v).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Turn on Bluetooth and pair");
        assert_eq!(
            items[0].value,
            Choice::Pair {
                radios: Some(Settings {
                    bluetooth_enabled: Some(true),
                    ..Default::default()
                })
            }
        );
        s.airplane_mode = Some(true);
        let items = phone_menu_of(&phone_view(s, PhoneView::default())).unwrap();
        assert_eq!(items[0].label, "Turn off airplane mode and pair");
        assert_eq!(items[0].detail.as_deref(), Some("Bluetooth turns on too"));
        assert_eq!(
            items[0].value,
            Choice::Pair {
                radios: Some(Settings {
                    bluetooth_enabled: Some(true),
                    airplane_mode: Some(false),
                    ..Default::default()
                })
            }
        );
        s.bluetooth_enabled = Some(true);
        let items = phone_menu_of(&phone_view(s, PhoneView::default())).unwrap();
        assert_eq!(items[0].detail, None);
        assert_eq!(
            items[0].value,
            Choice::Pair {
                radios: Some(Settings {
                    airplane_mode: Some(false),
                    ..Default::default()
                })
            }
        );
        s.airplane_mode = Some(false);
        let items = phone_menu_of(&phone_view(s, PhoneView::default())).unwrap();
        assert_eq!(items[0].label, "Start pairing");
        assert_eq!(items[0].detail.as_deref(), Some("2 minutes"));
        assert_eq!(items[0].value, Choice::Pair { radios: None });
        // Not before glassd's settings are known.
        let mut v = phone_view(s, PhoneView::default());
        v.settings = None;
        assert!(phone_menu_of(&v).is_none());
    }

    #[test]
    fn a_paired_phone_is_named_and_forgetting_asks_first() {
        let v = phone_view(
            settings_defaults(),
            PhoneView {
                paired: true,
                paired_name: "Pixel 8".into(),
                unsupported: false,
            },
        );
        let c = card_content(ids::PHONE, &v).unwrap();
        assert_eq!(subtitle(c.clone()), "Paired with Pixel 8");
        assert!(matches!(c, CardContent::Settings { title, .. } if title == "Phone"));
        let items = phone_menu_of(&v).unwrap();
        assert_eq!(items[0].label, "Forget the phone");
        assert_eq!(items[0].detail.as_deref(), Some("Pixel 8"));
        assert_eq!(items[0].value, Choice::AskForget);
        let q = forget_question(Locale::En);
        assert_eq!(q[0].value, Choice::Dismiss, "cancel comes first");
        assert_eq!(q[1].value, Choice::Forget);
        let q = paired_question(Locale::It, "Pixel 8");
        assert_eq!(q[0].label, "Associato a Pixel 8");
        assert_eq!(q[0].value, Choice::AskBond, "a reflex tap answers nothing");
        assert_eq!(q[1].label, "È il mio telefono");
        assert_eq!(q[1].value, Choice::KeepPhone);
        assert_eq!(q[2].label, "Non è il mio telefono");
        assert_eq!(q[2].value, Choice::Forget);
    }

    #[test]
    fn a_glassd_without_the_pairing_mode_disables_the_card() {
        let v = phone_view(
            settings_defaults(),
            PhoneView {
                unsupported: true,
                ..Default::default()
            },
        );
        assert_eq!(
            subtitle(card_content(ids::PHONE, &v).unwrap()),
            "Not available on this Glass"
        );
        assert!(phone_menu_of(&v).is_none());
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
