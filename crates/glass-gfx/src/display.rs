use crate::Frame;
use std::path::{Path, PathBuf};

/// A frame could not be shown.
#[derive(Debug)]
pub enum DisplayError {
    /// The frame does not have the display's size.
    SizeMismatch {
        expected: (u32, u32),
        got: (u32, u32),
    },
    /// The display is gone (for example, its window was closed).
    Closed,
    /// Writing the frame failed.
    Io(String),
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayError::SizeMismatch { expected, got } => write!(
                f,
                "frame is {}x{}, display is {}x{}",
                got.0, got.1, expected.0, expected.1
            ),
            DisplayError::Closed => write!(f, "display closed"),
            DisplayError::Io(e) => write!(f, "display I/O error: {e}"),
        }
    }
}

impl std::error::Error for DisplayError {}

/// Something that shows finished frames: a window, the framebuffer, a test buffer.
pub trait Display {
    /// The size in pixels a presented frame must have.
    fn size(&self) -> (u32, u32);

    /// Shows `frame`. The frame must be `size()` large.
    fn present(&mut self, frame: &Frame) -> Result<(), DisplayError>;
}

pub(crate) fn check_size(d: &dyn Display, frame: &Frame) -> Result<(), DisplayError> {
    let got = (frame.width(), frame.height());
    if got != d.size() {
        return Err(DisplayError::SizeMismatch {
            expected: d.size(),
            got,
        });
    }
    Ok(())
}

/// A display without a screen: keeps the last frame in memory, counts presents, and
/// optionally writes every frame as `frame-NNNNN.png` into a directory.
pub struct HeadlessDisplay {
    size: (u32, u32),
    last: Option<Frame>,
    presented: u64,
    out_dir: Option<PathBuf>,
}

impl HeadlessDisplay {
    pub fn new(width: u32, height: u32) -> HeadlessDisplay {
        HeadlessDisplay {
            size: (width, height),
            last: None,
            presented: 0,
            out_dir: None,
        }
    }

    /// Also writes each presented frame as a PNG into `dir` (created if missing).
    pub fn with_output_dir(mut self, dir: impl AsRef<Path>) -> Result<Self, DisplayError> {
        std::fs::create_dir_all(dir.as_ref()).map_err(|e| DisplayError::Io(e.to_string()))?;
        self.out_dir = Some(dir.as_ref().to_path_buf());
        Ok(self)
    }

    /// The most recently presented frame.
    pub fn last_frame(&self) -> Option<&Frame> {
        self.last.as_ref()
    }

    /// How many frames were presented.
    pub fn presented(&self) -> u64 {
        self.presented
    }
}

impl Display for HeadlessDisplay {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn present(&mut self, frame: &Frame) -> Result<(), DisplayError> {
        check_size(self, frame)?;
        if let Some(dir) = &self.out_dir {
            let path = dir.join(format!("frame-{:05}.png", self.presented));
            frame
                .write_png(&path)
                .map_err(|e| DisplayError::Io(e.to_string()))?;
        }
        match &mut self.last {
            Some(last) => last.data_mut().copy_from_slice(frame.data()),
            None => self.last = Some(frame.clone()),
        }
        self.presented += 1;
        Ok(())
    }
}
