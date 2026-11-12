//! The shortcuts a Windows listener's keyboard hook registers.

use super::*;

/// The shortcuts a listener's keyboard hook registers.
#[derive(PartialEq)]
pub(super) struct Shortcuts {
    pub(super) hotkey: WindowsHotkey,
    pub(super) paste_last_hotkey: Option<WindowsHotkey>,
    pub(super) raw_dictation_hotkey: Option<WindowsHotkey>,
    pub(super) terminal_paste_hotkey: Option<WindowsHotkey>,
    pub(super) second_dictation_hotkey: Option<WindowsHotkey>,
    pub(super) annotate_hotkey: Option<WindowsHotkey>,
    pub(super) cancel_hotkey: Option<WindowsHotkey>,
    pub(super) action_hotkey: Option<WindowsHotkey>,
    pub(super) stop_all_hotkey: Option<WindowsHotkey>,
    pub(super) double_tap_lock: bool,
    pub(super) double_tap_only: bool,
    pub(super) shortcut_mode: crate::dictation::ShortcutMode,
    pub(super) share_dictation_shortcut: bool,
}

impl Shortcuts {
    /// The shortcuts `config` registers, as the listener starts and restarts
    /// them when settings change.
    pub(super) fn from_config(config: &WindowsDictationConfig) -> Self {
        Self {
            hotkey: config.hotkey.clone(),
            paste_last_hotkey: config.paste_last_hotkey.clone(),
            raw_dictation_hotkey: config.raw_dictation_hotkey.clone(),
            terminal_paste_hotkey: config.terminal_paste_hotkey.clone(),
            second_dictation_hotkey: config.second_dictation_hotkey.clone(),
            annotate_hotkey: config.annotate_hotkey.clone(),
            cancel_hotkey: config.cancel_hotkey.clone(),
            action_hotkey: config.action_hotkey.clone(),
            stop_all_hotkey: config.stop_all_hotkey.clone(),
            double_tap_lock: config.double_tap_lock,
            double_tap_only: config.double_tap_only,
            shortcut_mode: config.shortcut_mode,
            share_dictation_shortcut: config.share_dictation_shortcut,
        }
    }

    pub(super) fn start(&self) -> Result<WindowsHotkeyMonitor> {
        WindowsHotkeyMonitor::start(crate::windows::input::HookConfig {
            binding: self.hotkey.clone(),
            paste_last: self.paste_last_hotkey.clone(),
            raw: self.raw_dictation_hotkey.clone(),
            terminal_paste: self.terminal_paste_hotkey.clone(),
            second: self.second_dictation_hotkey.clone(),
            annotate: self.annotate_hotkey.clone(),
            cancel: self.cancel_hotkey.clone(),
            action: self.action_hotkey.clone(),
            stop_all: self.stop_all_hotkey.clone(),
            double_tap_enabled: self.double_tap_lock,
            // Double-tap-only depends on the double-tap window; without the
            // lock there is no second tap to wait for.
            double_tap_only: self.double_tap_lock && self.double_tap_only,
            shortcut_mode: self.shortcut_mode,
            share_trigger: self.share_dictation_shortcut,
        })
    }
}
