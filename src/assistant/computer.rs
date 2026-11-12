//! The assistant's hands on the desktop (macOS): screenshots, pointer and
//! keyboard events, opening apps and URLs, and reading the frontmost app's
//! context through Accessibility.
//!
//! Coordinates: every `Action` takes points in the space of the latest
//! `Screenshot` (its `width` × `height`), which may be scaled down from the
//! display; this module maps them back to the display's points.
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

/// One thing to do on the desktop. Mirrors Anthropic's computer-use actions,
/// plus opening apps and URLs.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Screenshot,
    MouseMove { x: i32, y: i32 },
    LeftClick { x: i32, y: i32, modifiers: Option<String> },
    RightClick { x: i32, y: i32 },
    MiddleClick { x: i32, y: i32 },
    DoubleClick { x: i32, y: i32 },
    TripleClick { x: i32, y: i32 },
    LeftClickDrag { from: (i32, i32), to: (i32, i32) },
    /// Types text as if from the keyboard (any Unicode).
    Type { text: String },
    /// A key or chord in xdotool style: "Return", "cmd+c", "ctrl+shift+t".
    Key { keys: String },
    /// Holds `keys` for `seconds`.
    HoldKey { keys: String, seconds: f32 },
    Scroll {
        x: i32,
        y: i32,
        direction: ScrollDirection,
        amount: u32,
    },
    Wait { seconds: f32 },
    /// Opens or activates an app by name ("Safari") or bundle id.
    OpenApplication { name: String },
    OpenUrl { url: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl Action {
    /// A short line a person reads in the HUD and in Chats: "Clicking at
    /// 412, 88", "Typing “Hello…”", "Pressing ⌘C", "Opening Safari".
    pub fn describe(&self) -> String {
        format!("{self:?}")
    }
}

/// The size screenshots come in, which actions' coordinates use.
pub fn display_size() -> (u32, u32) {
    (1280, 800)
}

/// A screenshot of the display the pointer is on, scaled so its long side
/// is at most 1280 pixels.
pub fn screenshot() -> Result<Screenshot> {
    Err(eyre!("taking screenshots isn't available yet"))
}

/// Does `action`. A `Screenshot` action, and any action after which the
/// screen should be looked at again, returns the new screenshot.
pub fn perform(action: &Action) -> Result<Option<Screenshot>> {
    let _ = action;
    Err(eyre!("desktop control isn't available yet"))
}

/// What the desktop looks like now, for a question: a screenshot when
/// `with_screenshot`, the selected text, the frontmost app and its window.
pub fn context(with_screenshot: bool) -> crate::assistant::Context {
    let _ = with_screenshot;
    crate::assistant::Context::default()
}

/// Whether Hex may control the computer and record the screen (macOS
/// Accessibility and Screen Recording), for Chats to show what's missing.
pub fn permissions() -> Permissions {
    Permissions::default()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Permissions {
    pub accessibility: bool,
    pub screen_recording: bool,
}
