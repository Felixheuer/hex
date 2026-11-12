//! The ids Hex binds its shortcuts under in the shortcuts portal, and what
//! the desktop granted them, as Settings shows it.

use super::{DesktopGrantedShortcut, LinuxHotkey, Shortcut};

/// Whether a portal trigger holds modifiers: the one the desktop granted,
/// else the one Hex asked for.
pub(super) fn trigger_has_modifiers(granted: Option<&str>, requested: &LinuxHotkey) -> bool {
    match granted.map(str::trim).filter(|trigger| !trigger.is_empty()) {
        Some(trigger) => crate::linux::portal::readable_trigger(trigger).contains('+'),
        None => requested.control || requested.alt || requested.shift || requested.super_key,
    }
}

/// The portal id of a shortcut asking for `trigger`. A desktop keeps the
/// trigger it stored for an id and ignores a later `preferred_trigger`
/// (GNOME, KDE), so a shortcut changed in Hex gets a new id, which the
/// desktop binds to the new trigger (#575). An unchanged shortcut keeps its
/// id, and with it any trigger the user assigned in the desktop.
///
/// With `desktop_assigns` (Hyprland), the desktop ignores the trigger and
/// the user binds the id by name in its config, so the id is the bare
/// `base`: recording another key in Hex must not leave that bind pointing
/// at a name Hex no longer listens to (#1376).
pub(super) fn portal_shortcut_id(base: &str, trigger: &str, desktop_assigns: bool) -> String {
    if desktop_assigns {
        return base.to_string();
    }
    format!("{base}-{}", trigger.replace('+', "-"))
}

/// The name Hex 3.3 and earlier gave `shortcut` on Hyprland, the trigger in
/// the id (`dictation-ALT-space`), which configs written then still bind.
/// Hex binds it beside the bare id so those binds keep working while the
/// recorded key is the one they were written for. Only the shortcuts 3.3
/// had get one.
pub(super) fn hyprland_legacy_id(shortcut: Shortcut, trigger: &str) -> Option<String> {
    matches!(
        shortcut,
        Shortcut::Dictation | Shortcut::PasteLast | Shortcut::Capture | Shortcut::RawDictation
    )
    .then(|| portal_shortcut_id(shortcut.portal_id().0, trigger, false))
}

/// The trigger the desktop granted a shortcut, by the id Hex bound it under.
pub(super) fn granted_trigger<'a>(
    granted: &'a std::collections::HashMap<String, Option<String>>,
    ids: &[Option<String>; Shortcut::ALL.len()],
    shortcut: Shortcut,
) -> Option<&'a str> {
    ids[shortcut.index()]
        .as_deref()
        .and_then(|id| granted.get(id))
        .and_then(Option::as_deref)
}

/// What the desktop granted, as Settings shows it: each shortcut Hex asked
/// for. With `desktop_assigns` (Hyprland), a shortcut without a trigger is
/// unconfirmed rather than refused.
pub(super) fn project_grants(
    granted: &std::collections::HashMap<String, Option<String>>,
    requested: &[Option<(Shortcut, &LinuxHotkey)>],
    ids: &[Option<String>; Shortcut::ALL.len()],
    desktop_assigns: bool,
) -> Vec<DesktopGrantedShortcut> {
    requested
        .iter()
        .flatten()
        .map(|(shortcut, _)| {
            let trigger = granted_trigger(granted, ids, *shortcut)
                .map(crate::linux::portal::readable_trigger);
            DesktopGrantedShortcut {
                name: shortcut.portal_id().1.into(),
                unconfirmed: desktop_assigns && trigger.is_none(),
                trigger,
            }
        })
        .collect()
}
