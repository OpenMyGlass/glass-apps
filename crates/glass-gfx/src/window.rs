//! Host window back end (`window` feature), for development on a desktop.

use crate::display::check_size;
use crate::{Display, DisplayError, Frame};
use minifb::{KeyRepeat, Window, WindowOptions};

/// Keys the host window reports. Kept free of `minifb` types so callers do not depend on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Enter,
    Space,
    Escape,
}

/// A desktop window showing frames at their pixel size.
pub struct WindowDisplay {
    window: Window,
    size: (u32, u32),
    buffer: Vec<u32>,
}

impl WindowDisplay {
    pub fn new(title: &str, width: u32, height: u32) -> Result<WindowDisplay, DisplayError> {
        let window = Window::new(
            title,
            width as usize,
            height as usize,
            WindowOptions::default(),
        )
        .map_err(|e| DisplayError::Io(e.to_string()))?;
        Ok(WindowDisplay {
            window,
            size: (width, height),
            buffer: vec![0; (width * height) as usize],
        })
    }

    /// False once the user closed the window.
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    /// Processes window events without presenting a new frame.
    pub fn poll(&mut self) {
        self.window.update();
    }

    /// Keys pressed since the last call (no key repeat).
    pub fn keys_pressed(&self) -> Vec<Key> {
        self.window
            .get_keys_pressed(KeyRepeat::No)
            .into_iter()
            .filter_map(|k| match k {
                minifb::Key::Left => Some(Key::Left),
                minifb::Key::Right => Some(Key::Right),
                minifb::Key::Up => Some(Key::Up),
                minifb::Key::Down => Some(Key::Down),
                minifb::Key::Enter | minifb::Key::NumPadEnter => Some(Key::Enter),
                minifb::Key::Space => Some(Key::Space),
                minifb::Key::Escape => Some(Key::Escape),
                _ => None,
            })
            .collect()
    }
}

impl Display for WindowDisplay {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn present(&mut self, frame: &Frame) -> Result<(), DisplayError> {
        check_size(self, frame)?;
        if !self.window.is_open() {
            return Err(DisplayError::Closed);
        }
        // Display frames are opaque, so premultiplied bytes are the colour; drop alpha.
        for (dst, px) in self.buffer.iter_mut().zip(frame.data().chunks_exact(4)) {
            *dst = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        }
        self.window
            .update_with_buffer(&self.buffer, self.size.0 as usize, self.size.1 as usize)
            .map_err(|e| DisplayError::Io(e.to_string()))
    }
}
