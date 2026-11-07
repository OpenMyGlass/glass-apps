//! Every settings card and menu item, and the setup cards, in every language: no text is cut,
//! and every message of the catalogue fits on a card. `GLASS_I18N_SHEETS=DIR` also writes
//! contact sheets (half-size cards, four across) to check the screens by eye.

use glass_gfx::{Frame, Rect};
use glass_launcher::platform::RadioStatus;
use glass_launcher::settings::{self, ids, PhoneView, ServiceInfo, SettingsView};
use glass_launcher::setup::Setup;
use glass_ui::card::{render_card, text_is_cut};
use glass_ui::i18n::{Locale, Msg};
use glass_ui::{Card, CardContent, RenderContext};

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

fn screens(locale: Locale) -> Vec<CardContent> {
    let v = view(locale);
    let mut out = Vec::new();
    let mut setup = Setup::new();
    setup.set_locale(locale);
    out.push(setup.card());
    for card in settings::cards(&v) {
        out.push(card.content.clone());
        if let Some((items, _)) = settings::menu_for(&card, &v) {
            let count = items.len();
            for (index, it) in items.into_iter().enumerate() {
                out.push(CardContent::MenuItem {
                    label: it.label,
                    detail: it.detail,
                    index,
                    count,
                });
            }
        }
    }
    // The phone card (only with glassd's companion-link) in each of its states, with its menus.
    let mut airplane = settings::settings_defaults();
    airplane.airplane_mode = Some(true);
    let phones = [
        (airplane, PhoneView::default()),
        (settings::settings_defaults(), PhoneView::default()),
        (
            settings::settings_defaults(),
            PhoneView {
                paired: true,
                paired_name: "Galaxy S24 Ultra".into(),
                unsupported: false,
            },
        ),
        (
            settings::settings_defaults(),
            PhoneView {
                unsupported: true,
                ..Default::default()
            },
        ),
    ];
    for (s, phone) in phones {
        let mut v = view(locale);
        v.settings = Some(s);
        v.service = Some(ServiceInfo {
            version: "x".into(),
            capabilities: vec!["companion-link".into()],
        });
        v.phone = phone;
        let content = settings::card_content(ids::PHONE, &v).unwrap();
        out.push(content.clone());
        if let Some((items, _)) = settings::menu_for(&Card::new(ids::PHONE, content), &v) {
            push_menu(&mut out, items);
        }
    }
    push_menu(&mut out, settings::forget_question(locale));
    push_menu(
        &mut out,
        settings::paired_question(locale, "Galaxy S24 Ultra"),
    );
    out
}

fn push_menu(out: &mut Vec<CardContent>, items: Vec<glass_ui::MenuItem<settings::Choice>>) {
    let count = items.len();
    for (index, it) in items.into_iter().enumerate() {
        out.push(CardContent::MenuItem {
            label: it.label,
            detail: it.detail,
            index,
            count,
        });
    }
}

#[test]
fn no_screen_cuts_its_text_in_any_language() {
    for locale in Locale::ALL {
        for c in screens(locale) {
            assert!(!text_is_cut(&c, 640, 360), "{locale:?}: {c:?}");
        }
    }
}

#[test]
fn every_message_fits_on_a_card_in_every_language() {
    for locale in Locale::ALL {
        for m in Msg::ALL {
            let c = CardContent::settings(m.get(locale));
            assert!(!text_is_cut(&c, 640, 360), "{locale:?}: {m:?}");
        }
    }
}

#[test]
fn contact_sheets() {
    let Some(dir) = std::env::var_os("GLASS_I18N_SHEETS") else {
        return;
    };
    const PER_PAGE: usize = 20;
    for locale in Locale::ALL {
        for (page, cards) in screens(locale).chunks(PER_PAGE).enumerate() {
            let mut sheet = Frame::new(4 * 324, 5 * 184);
            sheet.fill(glass_gfx::Color::grey(0x40));
            for (i, c) in cards.iter().enumerate() {
                let f = render_card(
                    &Card::new(1, c.clone()),
                    &RenderContext::default(),
                    640,
                    360,
                );
                let (x, y) = ((i % 4) as f32 * 324.0 + 2.0, (i / 4) as f32 * 184.0 + 2.0);
                sheet.draw_image(&f, Rect::new(x, y, 320.0, 180.0));
            }
            sheet
                .write_png(std::path::Path::new(&dir).join(format!("{locale:?}-{page}.png")))
                .unwrap();
        }
    }
}
