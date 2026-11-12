//! What opens the window for the assistant, from any thread: its overlays,
//! its notifications, the Action shortcut, and the tray.

use std::sync::OnceLock;

use crate::rune_ui::{Destination, Ui};

/// The window's app, once it runs.
static APP: OnceLock<rune::AppHandle<Ui>> = OnceLock::new();

/// Keeps the window's app for [`open_chat`] and [`open_assistant_setup`].
pub(crate) fn set_window(app: rune::AppHandle<Ui>) {
    let _ = APP.set(app);
}

/// Opens the window on Chats at `conversation`, or on the most relevant
/// one with `None` (nothing while the assistant is off).
pub(crate) fn open_chat(conversation: Option<String>) {
    let Some(app) = APP.get() else {
        return;
    };
    app.update(move |ui, tx| crate::rune_ui::show(ui, Destination::Chats(conversation), tx));
    app.open();
}

/// Opens the onboarding where it was left: the voice first run while that
/// isn't done, else the assistant's pages at their saved page (a hold of
/// the Action shortcut before setup finished, or the bubble's "Continue
/// setup").
pub(crate) fn open_assistant_setup() {
    let Some(app) = APP.get() else {
        return;
    };
    app.update(crate::rune_ui::open_assistant_setup);
    app.open();
}

/// The assistant as the tray shows it: off (`None`), or whether a task runs
/// (Stop all tasks is enabled only then).
pub(crate) fn tray_state(settings: &crate::app_settings::AppSettings) -> Option<bool> {
    crate::assistant::enabled(settings).then(crate::assistant::busy)
}
