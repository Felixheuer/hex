//! What Hex itself reads from the desktop (macOS): a quick question's
//! context (screenshot, selected text, frontmost app) and where a task's
//! windows are, for their outlines. Tasks operate apps through agent-desktop
//! (`backends`), not through this module.
//!
//! OWNER: hands (with `speech`).

use color_eyre::{Result, eyre::eyre};

/// The screen as the model sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screenshot {
    pub png: Vec<u8>,
    /// Size of `png` in pixels: the coordinate space actions use.
    pub width: u32,
    pub height: u32,
}

/// A screenshot of the display the pointer is on, scaled so its long side
/// is at most 1280 pixels.
pub fn screenshot() -> Result<Screenshot> {
    Err(eyre!("taking screenshots isn't available yet"))
}

/// Where `window` is on screen, in global display points (x, y, width,
/// height), for its task's outline; `None` when it's gone or hidden.
pub fn window_frame(window: &crate::assistant::TaskWindow) -> Option<(f64, f64, f64, f64)> {
    let _ = window;
    None
}

/// What the desktop looks like now, for a question: a screenshot when
/// `with_screenshot`, the selected text, the frontmost app and its window.
pub fn context(with_screenshot: bool) -> crate::assistant::Context {
    let _ = with_screenshot;
    crate::assistant::Context::default()
}

/// Whether Hex may read other apps and record the screen (macOS
/// Accessibility and Screen Recording), for setup to show what's missing.
pub fn permissions() -> Permissions {
    Permissions::default()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Permissions {
    pub accessibility: bool,
    pub screen_recording: bool,
}
