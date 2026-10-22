//! Goldens of the settings cards, the menus they open and the wake-angle screen, rendered
//! in software (`tests/golden/settings-*.png`). With `GLASS_GLES_TESTS=1` and the `gles`
//! feature, the same cards are also rendered with GLES2 and compared with the same goldens
//! (within ±2, except the edge rule on the menu dots). The wake-angle screen is the
//! `adjust` golden of glass-ui.
//! `GLASS_UPDATE_GOLDEN=1` rewrites the software goldens.

use glass_gfx::golden::{self, Tolerance};
use glass_gfx::Frame;
use glass_launcher::platform::RadioStatus;
use glass_launcher::settings::{self, ids, ServiceInfo, SettingsView};
use glass_ui::card::render_card;
use glass_ui::{Card, CardContent, RenderContext};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ctx() -> RenderContext {
    RenderContext {
        now_s: 1_792_579_320,
        hour: 10,
        minute: 42,
    }
}

fn view() -> SettingsView {
    SettingsView {
        settings: Some(settings::settings_defaults()),
        wifi: RadioStatus::on(&["HomeNet"]),
        bluetooth: RadioStatus::off(),
        service: Some(ServiceInfo {
            version: "mock-0.1.0".into(),
            capabilities: vec![
                "mock".into(),
                "touchpad".into(),
                "screen".into(),
                "wear".into(),
            ],
        }),
        platform: "mock",
        renderer: "software",
    }
}

/// Every scene: name, card, tolerance on the GLES path.
/// The GLES tolerance of a card: strict, with the edge rule only on its rounded shapes.
fn gles_tolerance(card: &Card) -> Tolerance {
    let shapes = glass_ui::card::shape_rects(card, 640, 360);
    if shapes.is_empty() {
        Tolerance::Max(2)
    } else {
        Tolerance::Shapes(shapes)
    }
}

fn scenes() -> Vec<(String, Card, Tolerance)> {
    let v = view();
    let mut out: Vec<(String, Card, Tolerance)> = settings::cards(&v)
        .into_iter()
        .map(|c| {
            let name = match c.id {
                ids::WIFI => "wifi",
                ids::BLUETOOTH => "bluetooth",
                ids::SCREEN_TIMEOUT => "screen-timeout",
                ids::HEAD_WAKE => "head-wake",
                ids::WINK => "wink",
                ids::ON_HEAD => "on-head",
                ids::NUDGE => "nudge",
                ids::DEVICE => "device",
                _ => unreachable!(),
            };
            (format!("settings-{name}"), c, Tolerance::Max(2))
        })
        .collect();
    // The head-wake menu's second item and the timeout menu's current choice.
    let head = settings::cards(&v)
        .into_iter()
        .find(|c| c.id == ids::HEAD_WAKE)
        .unwrap();
    let (items, _) = settings::menu_for(&head, &v).unwrap();
    let card = menu_card(&items[1].label, items[1].detail.clone(), 1, items.len());
    out.push((
        "settings-menu-wake-angle".into(),
        card.clone(),
        gles_tolerance(&card),
    ));
    let timeout = settings::cards(&v)
        .into_iter()
        .find(|c| c.id == ids::SCREEN_TIMEOUT)
        .unwrap();
    let (items, first) = settings::menu_for(&timeout, &v).unwrap();
    out.push((
        "settings-menu-timeout".into(),
        menu_card(
            &items[first].label,
            items[first].detail.clone(),
            first,
            items.len(),
        ),
        Tolerance::Edges,
    ));
    out
}

fn menu_card(label: &str, detail: Option<String>, index: usize, count: usize) -> Card {
    Card::new(
        index as u64,
        CardContent::MenuItem {
            label: label.into(),
            detail,
            index,
            count,
        },
    )
}

fn check(name: &str, frame: &Frame, tol: Tolerance, actual: &str, update: bool) {
    let d = golden::check(
        &root().join("tests/golden"),
        &root().join("target").join(actual),
        name,
        frame,
        &tol,
        update,
    );
    println!("scene {name}: {d:?}");
}

#[test]
fn settings_goldens_software() {
    let update = std::env::var_os("GLASS_UPDATE_GOLDEN").is_some();
    for (name, card, _) in scenes() {
        let f = render_card(&card, &ctx(), 640, 360);
        check(&name, &f, Tolerance::Max(3), "golden-actual", update);
    }
}

#[cfg(feature = "gles")]
#[test]
fn settings_goldens_gles() {
    use glass_gfx::{Color, Compositor, Painter, Placement};
    use glass_gfx_gles::{GlesCompositor, GlesOptions};
    if std::env::var("GLASS_GLES_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: set GLASS_GLES_TESTS=1 to run the GLES tests");
        return;
    }
    let mut c = GlesCompositor::offscreen(
        640,
        360,
        GlesOptions {
            sgx540_checks: true,
            ..GlesOptions::default()
        },
    )
    .expect("GLES context");
    for (name, card, tol) in scenes() {
        let id = c.create_layer(640, 360, true).unwrap();
        c.paint_layer(id, &mut |p: &mut dyn Painter| {
            glass_ui::card::paint_card(&card, &ctx(), p)
        })
        .unwrap();
        c.compose(Color::BLACK, &[Placement::at(id, 0, 0)]).unwrap();
        let f = c.read_frame().unwrap();
        c.drop_layer(id);
        check(&name, &f, tol, "golden-actual-gles", false);
    }
}
