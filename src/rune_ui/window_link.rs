//! Opens Hex's window from any thread, on every OS: Chats on a
//! conversation (the HUD's task stack and "Open in Chats", a notification's
//! click, the tray), the assistant's setup (a hold of the Action shortcut
//! before it's done), or just the window. macOS goes through `macos_rune`,
//! which also brings the app forward; Linux and Windows through the
//! window's app handle, remembered as the app starts ([`remember`]).

#[cfg(not(target_os = "macos"))]
use std::sync::OnceLock;

#[cfg(not(target_os = "macos"))]
use super::{Destination, Ui};

#[cfg(not(target_os = "macos"))]
static WINDOW: OnceLock<rune::AppHandle<Ui>> = OnceLock::new();

/// Remembers the window's app handle (Linux and Windows), once.
#[cfg(not(target_os = "macos"))]
pub(crate) fn remember(window: rune::AppHandle<Ui>) {
    let _ = WINDOW.set(window);
}

/// Opens the window on Chats, on `conversation` when given.
pub(crate) fn open_chat(conversation: Option<String>) {
    #[cfg(target_os = "macos")]
    crate::macos_rune::open_chat(conversation);
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = WINDOW.get() {
        window.open();
        window.update(move |ui, tx| super::show(ui, Destination::Chats(conversation), tx));
    }
}

/// Opens the assistant's setup where it was left.
pub(crate) fn open_assistant_setup() {
    #[cfg(target_os = "macos")]
    crate::macos_rune::open_assistant_setup();
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = WINDOW.get() {
        window.open();
        window.update(|ui, tx| super::open_assistant_setup(ui, tx));
    }
}

/// Opens the window in front.
pub(crate) fn bring_forward() {
    #[cfg(target_os = "macos")]
    crate::macos_rune::bring_window_forward();
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = WINDOW.get() {
        window.open();
    }
}

/// Wires every way the assistant opens the window: the overlays' "Open in
/// Chats" and "Continue setup", a hold of the Action shortcut before setup,
/// and a notification's click. Linux and Windows; macOS wires its own in
/// `macos_rune`.
#[cfg(not(target_os = "macos"))]
pub(crate) fn wire_assistant() {
    super::assistant_overlay::set_open_chats(|id| open_chat(Some(id.to_owned())));
    super::assistant_overlay::set_open_setup(open_assistant_setup);
    crate::assistant::shortcut::set_open_setup(|| {
        super::assistant_overlay::setup_needed();
        open_assistant_setup();
    });
    crate::assistant::notify::set_open(|id| open_chat(Some(id.to_owned())));
}
