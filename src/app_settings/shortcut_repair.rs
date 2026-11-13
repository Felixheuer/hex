//! Shortcut keys each OS allows: Menu, Pause, or Scroll Lock alone (Linux),
//! a navigation key alone (Windows), the ISO section key alone (Linux and
//! macOS), shortcuts without a key in double-tap mode (macOS), and the
//! repairs that fold the persisted double-tap mode and the Action shortcut.

use super::hotkeys::ISO_SECTION_KEY_CODE;
use super::{AppSettings, HotkeyBinding, HotkeyKey, MAX_EXTRA_DICTATION_SHORTCUTS};

impl HotkeyKey {
    /// Menu, Pause, or Scroll Lock: keys that type nothing, which a Linux
    /// shortcut may use alone (issue 767).
    pub fn is_special(&self) -> bool {
        !self.is_mouse() && Self::special_name(&self.label)
    }

    /// Whether `name` names Menu, Pause, or Scroll Lock.
    pub fn special_name(name: &str) -> bool {
        is_scroll_lock(name)
            || matches!(name.trim().to_ascii_lowercase().as_str(), "menu" | "pause")
    }

    /// The § key, left of 1 on ISO keyboards, which Section key
    /// alone lets a shortcut use on its own (issue 1050). macOS knows it by
    /// virtual key code too, because a dead-key layout labels it `^`, `´`,
    /// or `` ` `` rather than §.
    pub fn is_section(&self) -> bool {
        !self.is_mouse()
            && (Self::section_name(&self.label)
                || (cfg!(target_os = "macos") && self.code == ISO_SECTION_KEY_CODE))
    }

    /// Whether `name` names the § key: `§` or `section`.
    pub fn section_name(name: &str) -> bool {
        let name = name.trim();
        name == "§" || name.eq_ignore_ascii_case("section")
    }

    /// Insert, Home, End, Page Up, or Page Down: keys that type nothing,
    /// which Special keys alone lets a Windows shortcut use alone
    /// (issue 794).
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn is_navigation(&self) -> bool {
        !self.is_mouse() && Self::navigation_name(&self.label)
    }

    /// Whether `name` names Insert, Home, End, Page Up, or Page Down.
    pub fn navigation_name(name: &str) -> bool {
        matches!(
            name.trim()
                .to_ascii_lowercase()
                .replace([' ', '_'], "")
                .as_str(),
            "insert" | "home" | "end" | "pageup" | "pagedown"
        )
    }
}

impl HotkeyBinding {
    /// The § key alone (`HotkeyKey::is_section`).
    #[cfg(test)]
    pub fn is_lone_section_key(&self) -> bool {
        self.modifiers.is_empty() && self.key.as_ref().is_some_and(HotkeyKey::is_section)
    }
}

/// `scrolllock`, `scroll_lock`, or `Scroll Lock`.
pub(super) fn is_scroll_lock(name: &str) -> bool {
    name.trim()
        .replace([' ', '_'], "")
        .eq_ignore_ascii_case("scrolllock")
}

impl AppSettings {
    /// On Linux a shortcut may use Menu, Pause, or Scroll Lock alone; on
    /// Windows, which takes those anyway, a navigation key alone.
    pub fn special_keys_allowed(&self) -> bool {
        cfg!(any(target_os = "linux", target_os = "windows"))
    }

    /// Double-tap shortcut mode accepts a dictation shortcut without a key
    /// (macOS).
    pub fn double_tap_modifiers_allowed(&self) -> bool {
        cfg!(target_os = "macos")
    }

    /// Clears a double-tap-only choice a shortcut can't support: the lock
    /// is off, or a lone modifier where the OS doesn't allow one. Each extra
    /// dictation shortcut row folds the same way.
    pub(crate) fn normalize_double_tap_settings(&mut self) {
        let modifiers_allowed = self.double_tap_modifiers_allowed();
        if !self.double_tap_lock || (self.dictation_hotkey.key.is_none() && !modifiers_allowed) {
            self.double_tap_only = false;
        }
        for row in &mut self.extra_dictation_hotkeys {
            row.mode = row
                .mode
                .for_binding(row.binding.as_ref(), modifiers_allowed);
        }
    }

    /// The extra dictation shortcuts keep at most
    /// [`MAX_EXTRA_DICTATION_SHORTCUTS`] rows, and each row's mode folds to
    /// one its binding can trigger (double-tap only needs a key, or macOS).
    pub(crate) fn repair_extra_dictation_shortcuts(&mut self) {
        self.extra_dictation_hotkeys
            .truncate(MAX_EXTRA_DICTATION_SHORTCUTS);
        let modifiers_allowed = self.double_tap_modifiers_allowed();
        for row in &mut self.extra_dictation_hotkeys {
            row.mode = row
                .mode
                .for_binding(row.binding.as_ref(), modifiers_allowed);
        }
    }

    /// The assistant's Action and Stop all tasks shortcuts never share a
    /// key with another active shortcut: while the assistant is on, one that
    /// does (the dictation shortcut moved onto right Command while it was
    /// off, say) is unbound, so General shows an empty recorder rather than
    /// a shortcut that does nothing. Off, they stay as saved. Returns
    /// whether either changed.
    pub(crate) fn repair_action_shortcut(&mut self) -> bool {
        if !crate::assistant::enabled(self) {
            return false;
        }
        let shared = self
            .assistant
            .action_hotkey
            .as_ref()
            .is_some_and(|binding| self.hotkey_in_use(super::HotkeySlot::Action, binding));
        if shared {
            tracing::info!("unbound the Action shortcut: another shortcut uses its keys");
            self.assistant.action_hotkey = None;
        }
        // Stop all tasks the same way, checked after the Action shortcut
        // has given up any key it shared.
        let stop_shared = self
            .assistant
            .stop_all_hotkey
            .as_ref()
            .is_some_and(|binding| self.hotkey_in_use(super::HotkeySlot::StopAll, binding));
        if stop_shared {
            tracing::info!("unbound Stop all tasks: another shortcut uses its keys");
            self.assistant.stop_all_hotkey = None;
        }
        shared || stop_shared
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::HotkeyModifiers;

    #[test]
    fn double_tap_only_on_a_lone_modifier_is_macos_only() {
        let binding = |key: Option<HotkeyKey>| HotkeyBinding {
            modifiers: HotkeyModifiers::option(),
            key,
        };
        let mut settings = AppSettings {
            dictation_hotkey: binding(None),
            double_tap_only: true,
            ..Default::default()
        };
        settings.normalize_double_tap_settings();
        assert_eq!(settings.double_tap_only, cfg!(target_os = "macos"));
        // A keyed binding keeps the choice everywhere.
        settings.dictation_hotkey = binding(Some(HotkeyKey::named("space")));
        settings.double_tap_only = true;
        settings.normalize_double_tap_settings();
        assert!(settings.double_tap_only);
    }

    #[test]
    fn menu_pause_and_scroll_lock_are_the_special_keys() {
        for name in ["menu", "Pause", "scrolllock", "scroll_lock", "Scroll Lock"] {
            assert!(HotkeyKey::special_name(name), "{name}");
            assert!(HotkeyKey::named(name).is_special(), "{name}");
        }
        assert_eq!(HotkeyKey::named("scrolllock").label, "Scroll Lock");
        for name in ["f9", "space", "m", "mouse4"] {
            assert!(!HotkeyKey::named(name).is_special(), "{name}");
        }
        assert_eq!(
            AppSettings::default().special_keys_allowed(),
            cfg!(any(target_os = "linux", target_os = "windows"))
        );
    }

    /// Issue 1050: the ISO section key alone may be a shortcut on Linux and
    /// macOS; § with a modifier is never "alone".
    #[test]
    fn the_section_key_alone_is_recognised() {
        let section = HotkeyBinding {
            modifiers: Default::default(),
            key: Some(HotkeyKey::named("section")),
        };
        assert!(section.is_lone_section_key());
        assert_eq!(HotkeyKey::named("section").label, "§");
        assert!(HotkeyKey::named("§").is_section());
        assert!(!HotkeyKey::named("s").is_section());
        let with_control = HotkeyBinding {
            modifiers: super::super::HotkeyModifiers::control_option(),
            key: Some(HotkeyKey::named("§")),
        };
        assert!(!with_control.is_lone_section_key());
    }

    /// macOS labels the ISO section key by what it types: a dead-key layout
    /// gives `^` for key code 10, and the section-key rules must still see
    /// it. Elsewhere only the label names the section key.
    #[test]
    fn a_dead_key_label_is_the_section_key_only_on_macos() {
        let dead_key = HotkeyBinding {
            modifiers: Default::default(),
            key: Some(HotkeyKey {
                code: ISO_SECTION_KEY_CODE,
                label: "^".into(),
            }),
        };
        assert_eq!(dead_key.is_lone_section_key(), cfg!(target_os = "macos"));
    }
}
