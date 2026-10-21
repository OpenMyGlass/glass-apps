//! Renderer choice: `--renderer gles|software`, with no silent fallback on the device.
//!
//! - Device builds (Android, Linux on ARM) with GLES support: the default is `gles`, and an
//!   EGL or GL failure is a hard error; `software` must be asked for.
//! - Host builds with GLES support: the default is `gles` when EGL initializes, else
//!   `software`, with a log line.
//! - Builds without the `gles` feature (the static musl debug build, where `dlopen` cannot
//!   work): `software` only; asking for `gles` is an error.

use glass_gfx::{Compositor, SoftwareCompositor, GLASS_HEIGHT, GLASS_WIDTH};

/// A renderer named on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renderer {
    Gles,
    Software,
}

impl std::str::FromStr for Renderer {
    type Err = String;

    fn from_str(s: &str) -> Result<Renderer, String> {
        match s {
            "gles" => Ok(Renderer::Gles),
            "software" => Ok(Renderer::Software),
            _ => Err(format!("--renderer must be gles or software, not {s}")),
        }
    }
}

/// Whether this is a device build (no fallback from GLES).
pub const DEVICE_BUILD: bool = cfg!(any(
    target_os = "android",
    all(
        target_os = "linux",
        any(target_arch = "arm", target_arch = "aarch64")
    )
));

/// Whether this build has the GLES renderer.
pub const GLES_BUILT: bool = cfg!(feature = "gles");

/// The build properties the choice depends on.
#[derive(Clone, Copy, Debug)]
pub struct Build {
    pub device: bool,
    pub gles: bool,
}

impl Build {
    pub fn this() -> Build {
        Build {
            device: DEVICE_BUILD,
            gles: GLES_BUILT,
        }
    }
}

/// Picks the compositor. `gles` creates the GLES one. Returns it with an optional line to
/// log (why software is used).
pub fn choose(
    requested: Option<Renderer>,
    build: Build,
    gles: impl FnOnce() -> Result<Box<dyn Compositor>, String>,
    software: impl FnOnce() -> Box<dyn Compositor>,
) -> Result<(Box<dyn Compositor>, Option<String>), String> {
    match requested {
        Some(Renderer::Software) => Ok((software(), None)),
        Some(Renderer::Gles) if !build.gles => {
            Err("this build has no GLES renderer (built without the `gles` feature)".into())
        }
        Some(Renderer::Gles) => gles()
            .map(|c| (c, None))
            .map_err(|e| format!("GLES renderer failed: {e}")),
        None if !build.gles => Ok((software(), None)),
        None if build.device => gles().map(|c| (c, None)).map_err(|e| {
            format!("GLES renderer failed: {e} (use --renderer software for the debug renderer)")
        }),
        None => match gles() {
            Ok(c) => Ok((c, None)),
            Err(e) => Ok((
                software(),
                Some(format!("GLES unavailable ({e}); rendering in software")),
            )),
        },
    }
}

/// The GLES compositor of the Glass display size, off screen (frames are read back).
#[cfg(feature = "gles")]
pub fn gles_compositor() -> Result<Box<dyn Compositor>, String> {
    let c = glass_gfx_gles::GlesCompositor::offscreen(
        GLASS_WIDTH,
        GLASS_HEIGHT,
        glass_gfx_gles::GlesOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    eprintln!("glass-launcher: gles: {}", c.info());
    Ok(Box::new(c))
}

#[cfg(not(feature = "gles"))]
pub fn gles_compositor() -> Result<Box<dyn Compositor>, String> {
    Err("built without the `gles` feature".into())
}

pub fn software_compositor() -> Box<dyn Compositor> {
    Box::new(SoftwareCompositor::new(GLASS_WIDTH, GLASS_HEIGHT))
}

/// [`choose`] for this build with the real renderers; logs the fallback line.
pub fn compositor_for(requested: Option<Renderer>) -> Result<Box<dyn Compositor>, String> {
    let (c, note) = choose(
        requested,
        Build::this(),
        gles_compositor,
        software_compositor,
    )?;
    if let Some(n) = note {
        eprintln!("glass-launcher: {n}");
    }
    eprintln!("glass-launcher: renderer {}", c.name());
    Ok(c)
}
