//! STUB, until the trigger's shortcut checks land: what else takes a
//! shortcut (`conflicts`), and the live "press your shortcut now" probe.
//! The setup flow's Shortcut page is built against these shapes; at
//! integration each body here is replaced by a call into the trigger's
//! functions, and this file goes away. Nothing here pretends: the conflict
//! check reports that it can't run yet, and the probe never hears a press.

use crate::app_settings::{AppSettings, HotkeySlot};

/// Something else that takes the shortcut first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) struct Conflict {
    /// Who takes it, for a person: "Siri", "Dictation", "Raycast".
    pub(in crate::rune_ui) owner: String,
    /// What it does with the keys: "Press right ⌘ twice starts Siri".
    pub(in crate::rune_ui) detail: String,
    pub(in crate::rune_ui) fix: Option<ConflictFix>,
}

/// What clears a conflict.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // The real check offers every fix.
pub(in crate::rune_ui) enum ConflictFix {
    /// System Settings at the page that owns it (an
    /// `x-apple.systempreferences:` link).
    OpenSettings(String),
    /// Quit the app that takes it (its name).
    QuitApp(String),
    /// Choose another shortcut in General.
    ChooseAnother,
}

/// STUB for the trigger's `conflicts(slot, &settings)`: every app and macOS
/// feature that takes `slot`'s binding. Off the window's thread.
pub(in crate::rune_ui) fn conflicts(
    slot: HotkeySlot,
    settings: &AppSettings,
) -> Result<Vec<Conflict>, String> {
    let _ = (slot, settings);
    Err(crate::desktop::i18n::tr("Hex can't check for other apps on the shortcut yet").into())
}

/// What the probe has heard since it started.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::rune_ui) struct Heard {
    /// Hex received a hold of the shortcut.
    pub(in crate::rune_ui) hold: bool,
    /// Hex received a double-tap of it.
    pub(in crate::rune_ui) double_tap: bool,
}

/// STUB for the trigger's probe: from now on, presses of `slot` are
/// recorded (and not acted on) until [`stop_probe`].
pub(in crate::rune_ui) fn start_probe(slot: HotkeySlot) {
    let _ = slot;
}

/// STUB: what the probe heard since [`start_probe`].
pub(in crate::rune_ui) fn probe(slot: HotkeySlot) -> Heard {
    let _ = slot;
    Heard::default()
}

/// STUB: the shortcut acts again.
pub(in crate::rune_ui) fn stop_probe(slot: HotkeySlot) {
    let _ = slot;
}
