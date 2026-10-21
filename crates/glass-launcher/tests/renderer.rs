use glass_gfx::{Compositor, SoftwareCompositor};
use glass_launcher::renderer::{choose, Build, Renderer};

fn soft() -> Box<dyn Compositor> {
    Box::new(SoftwareCompositor::new(4, 4))
}

/// A stand-in for a working GLES compositor (named differently by its size).
fn fake_gles() -> Result<Box<dyn Compositor>, String> {
    Ok(Box::new(SoftwareCompositor::new(8, 8)))
}

fn failing_gles() -> Result<Box<dyn Compositor>, String> {
    Err("no EGL".into())
}

const DEVICE: Build = Build {
    device: true,
    gles: true,
};
const HOST: Build = Build {
    device: false,
    gles: true,
};
const SOFTWARE_ONLY: Build = Build {
    device: true,
    gles: false,
};

#[test]
fn parses_the_option() {
    assert_eq!("gles".parse::<Renderer>(), Ok(Renderer::Gles));
    assert_eq!("software".parse::<Renderer>(), Ok(Renderer::Software));
    assert!("vulkan".parse::<Renderer>().is_err());
}

#[test]
fn device_default_is_gles_and_a_failure_is_an_error() {
    let (c, note) = choose(None, DEVICE, fake_gles, soft).unwrap();
    assert_eq!(c.size(), (8, 8));
    assert!(note.is_none());
    let e = choose(None, DEVICE, failing_gles, soft).err().unwrap();
    assert!(
        e.contains("no EGL") && e.contains("--renderer software"),
        "{e}"
    );
}

#[test]
fn host_default_falls_back_to_software_with_a_note() {
    let (c, note) = choose(None, HOST, failing_gles, soft).unwrap();
    assert_eq!(c.size(), (4, 4));
    assert!(note.unwrap().contains("no EGL"));
    let (c, _) = choose(None, HOST, fake_gles, soft).unwrap();
    assert_eq!(c.size(), (8, 8));
}

#[test]
fn asking_for_gles_never_falls_back() {
    for b in [DEVICE, HOST] {
        assert!(choose(Some(Renderer::Gles), b, failing_gles, soft).is_err());
    }
    assert!(choose(Some(Renderer::Gles), SOFTWARE_ONLY, fake_gles, soft).is_err());
}

#[test]
fn software_when_asked_or_when_the_build_has_no_gles() {
    for b in [DEVICE, HOST, SOFTWARE_ONLY] {
        let (c, _) = choose(Some(Renderer::Software), b, fake_gles, soft).unwrap();
        assert_eq!(c.size(), (4, 4));
    }
    let (c, note) = choose(None, SOFTWARE_ONLY, fake_gles, soft).unwrap();
    assert_eq!(c.size(), (4, 4));
    assert!(note.is_none());
}
