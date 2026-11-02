//! Per-locale golden images: three key screens rendered in each of the four languages, so a
//! change to a translation, the layout or the font shows up as an image diff. Goldens live
//! in `tests/golden/i18n/<tag>-<scene>.png`. Regenerate with
//! `GLASS_UPDATE_GOLDEN=1 cargo test -p glass-launcher --test golden_i18n`.
//! On a mismatch the actual image is written to `target/golden-actual/`.

use glass_gfx::golden::{self, Tolerance};
use glass_launcher::platform::RadioStatus;
use glass_launcher::settings::{self, ids, SettingsView};
use glass_launcher::setup::Setup;
use glass_ui::card::render_card;
use glass_ui::i18n::Locale;
use glass_ui::{Card, RenderContext};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ctx(locale: Locale) -> RenderContext {
    RenderContext {
        now_s: 1_792_579_320,
        hour: 10,
        minute: 42,
        locale,
    }
}

fn view(locale: Locale) -> SettingsView {
    SettingsView {
        settings: Some(settings::settings_defaults()),
        wifi: RadioStatus::on(&["HomeNet"]),
        bluetooth: RadioStatus::off(),
        service: None,
        platform: "mock",
        renderer: "software",
        locale,
    }
}

/// The three scenes for a locale: the setup welcome card, the scroll-to-pick language card
/// (highlighting this locale), and the screen-timeout settings card.
fn scenes(locale: Locale) -> Vec<(&'static str, Card)> {
    let mut welcome = Setup::new();
    welcome.set_locale(locale);
    let timeout = Card::new(
        ids::SCREEN_TIMEOUT,
        settings::card_content(ids::SCREEN_TIMEOUT, &view(locale)).unwrap(),
    );
    vec![
        ("welcome", Card::new(0, welcome.card())),
        (
            "language-picker",
            Card::new(0, Setup::with_language_picker(locale).card()),
        ),
        ("screen-timeout", timeout),
    ]
}

fn check(name: &str, card: &Card, locale: Locale, update: bool) {
    // Only the language picker (a menu card) has rounded shapes (the position dots); hold
    // everything else to the strict tolerance, the dots to the edge rule.
    let shapes = glass_ui::card::shape_rects(card, 640, 360);
    let tol = if shapes.is_empty() {
        Tolerance::Max(3)
    } else {
        Tolerance::Shapes(shapes)
    };
    let frame = render_card(card, &ctx(locale), 640, 360);
    let d = golden::check(
        &root().join("tests/golden/i18n"),
        &root().join("target/golden-actual"),
        name,
        &frame,
        &tol,
        update,
    );
    println!("scene {name}: {d:?}");
}

#[test]
fn locale_goldens() {
    let update = std::env::var_os("GLASS_UPDATE_GOLDEN").is_some();
    for locale in Locale::ALL {
        for (scene, card) in scenes(locale) {
            check(&format!("{}-{scene}", locale.tag()), &card, locale, update);
        }
    }
}
