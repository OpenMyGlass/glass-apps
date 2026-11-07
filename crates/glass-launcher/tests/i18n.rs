//! Internationalisation tests: the bundled font covers every character the four locales
//! can show, key screens read correctly in each language, and changing the locale at runtime
//! re-renders the UI (see `golden_i18n.rs` for the per-locale images).

use glass_gfx::{fonts, Weight};
use glass_launcher::dictation::{Dictation, Failure};
use glass_launcher::platform::RadioStatus;
use glass_launcher::settings::{self, ids, SettingsView};
use glass_launcher::setup::{Setup, Step};
use glass_ui::i18n::{Locale, Msg};
use glass_ui::CardContent;

/// Every character any locale can put on screen: all catalogue strings and the endonyms.
fn displayable_chars() -> std::collections::BTreeSet<char> {
    let mut set = std::collections::BTreeSet::new();
    for &m in Msg::ALL {
        for l in Locale::ALL {
            set.extend(m.get(l).chars());
        }
    }
    for l in Locale::ALL {
        set.extend(l.endonym().chars());
    }
    set
}

/// The single bundled Inter font must have a glyph for every character the UI can render in
/// Italian, English, French and Spanish — the whole point of "one font covers all four".
/// This is the guard against a missing accent (à é è ì ò ù ñ ç …), inverted punctuation
/// (¿ ¡) and the degree sign.
#[test]
fn the_font_covers_every_character_of_every_locale() {
    let font = fonts::font(Weight::Light);
    let mut missing = Vec::new();
    for ch in displayable_chars() {
        // '{' and '}' only ever appear inside placeholders, which are filled before drawing;
        // a real newline never reaches layout. Everything else is drawn.
        if ch == '{' || ch == '}' {
            continue;
        }
        if font.lookup_glyph_index(ch) == 0 && ch != '\u{0}' {
            missing.push(ch);
        }
    }
    assert!(
        missing.is_empty(),
        "Inter-Light is missing glyphs for: {missing:?}"
    );
    // Spot-check the accents and inverted punctuation the languages rely on.
    for ch in "àáâèéêëìíîïòóôùúûñçÀÉÈÌÒÙÑÇ°¿¡…".chars() {
        assert_ne!(font.lookup_glyph_index(ch), 0, "no glyph for {ch:?}");
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
        phone: Default::default(),
    }
}

fn settings_subtitle(id: glass_ui::CardId, locale: Locale) -> String {
    match settings::card_content(id, &view(locale)).unwrap() {
        CardContent::Settings {
            subtitle: Some(s), ..
        } => s,
        c => panic!("{c:?}"),
    }
}

fn settings_title(id: glass_ui::CardId, locale: Locale) -> String {
    match settings::card_content(id, &view(locale)).unwrap() {
        CardContent::Settings { title, .. } => title,
        CardContent::Info { title, .. } => title,
        c => panic!("{c:?}"),
    }
}

fn welcome_lines(locale: Locale) -> (String, Vec<String>) {
    let mut s = Setup::new();
    s.set_locale(locale);
    assert_eq!(s.step(), &Step::Welcome);
    match s.card() {
        CardContent::Info { title, lines } => (title, lines),
        c => panic!("{c:?}"),
    }
}

/// Key screens read correctly in each of the four languages (the "right strings per locale,
/// 4x" check at the string level; the golden images are in `golden_i18n.rs`).
#[test]
fn key_screens_read_correctly_in_each_locale() {
    // Settings: the Wi-Fi title, the screen-timeout value and the Wink status.
    assert_eq!(settings_title(ids::WIFI, Locale::It), "Wi-Fi");
    assert_eq!(
        settings_subtitle(ids::SCREEN_TIMEOUT, Locale::It),
        "2 minuti"
    );
    assert_eq!(
        settings_subtitle(ids::SCREEN_TIMEOUT, Locale::En),
        "2 minutes"
    );
    assert_eq!(
        settings_subtitle(ids::SCREEN_TIMEOUT, Locale::Fr),
        "2 minutes"
    );
    assert_eq!(
        settings_subtitle(ids::SCREEN_TIMEOUT, Locale::Es),
        "2 minutos"
    );
    assert_eq!(settings_subtitle(ids::NUDGE, Locale::Fr), "Désactivé");
    // The language card names each language in its own language.
    assert_eq!(settings_title(ids::LANGUAGE, Locale::Es), "Idioma");
    assert_eq!(settings_subtitle(ids::LANGUAGE, Locale::Es), "Español");

    // Setup welcome.
    assert_eq!(welcome_lines(Locale::It).0, "Benvenuto");
    assert_eq!(welcome_lines(Locale::En).0, "Welcome");
    assert_eq!(welcome_lines(Locale::Fr).0, "Bienvenue");
    assert_eq!(welcome_lines(Locale::Es).0, "Bienvenido");
    assert_eq!(welcome_lines(Locale::It).1[1], "Tocca per iniziare");

    // Dictation failure.
    let failed = |f: Failure, l| f.text(l);
    assert_eq!(
        failed(Failure::LinkLost, Locale::Es),
        "Se perdió el enlace al teléfono"
    );
    assert_eq!(
        failed(Failure::NothingHeard, Locale::Fr),
        "Rien n'a été entendu"
    );

    // A listening dictation card.
    let mut d = Dictation::new(1);
    d.on_state(&glass_api::proto::DictationState {
        request_id: 1,
        state: glass_api::proto::dictation_state::State::Listening as i32,
        error: String::new(),
    });
    match d.card(Locale::It, 0) {
        CardContent::Text { body, source, .. } => {
            assert_eq!(body, "Parla ora");
            assert_eq!(source, "In ascolto");
        }
        c => panic!("{c:?}"),
    }
}
