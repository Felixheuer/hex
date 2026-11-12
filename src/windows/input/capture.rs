//! One capture's state as the keyboard hook drives it: held, locked by a
//! double tap or Toggle, and which shortcut owns it.

use super::{DOUBLE_TAP_WINDOW_MS, DictationFlavour, HotkeyAction};

#[derive(Default)]
pub(super) struct CaptureState {
    pub(super) active: bool,
    pub(super) locked: bool,
    pub(super) second_tap: bool,
    pub(super) recording: bool,
    pub(super) last_release_ms: Option<u32>,
    /// Every press acts as the second tap of a double-tap: releasing it
    /// locks the capture, and the next press finishes it.
    pub(super) toggle: bool,
    /// The shortcut, not another, owns this capture (or the last one).
    pub(super) flavour: DictationFlavour,
}

impl CaptureState {
    /// Whether another shortcut owns a capture in progress.
    pub(super) fn owned_by_other(&self, flavour: DictationFlavour) -> bool {
        self.flavour != flavour && (self.active || self.locked)
    }

    /// Hands an idle capture to one shortcut; a tap of the other one never
    /// counts toward its double tap.
    pub(super) fn claim(&mut self, flavour: DictationFlavour) {
        if self.flavour != flavour && !self.active && !self.locked {
            self.flavour = flavour;
            self.last_release_ms = None;
        }
    }

    pub(super) fn press(
        &mut self,
        double_tap_enabled: bool,
        double_tap_only: bool,
        timestamp_ms: u32,
    ) -> Option<HotkeyAction> {
        if self.locked {
            self.locked = false;
            self.recording = false;
            self.last_release_ms = None;
            return Some(HotkeyAction::Finish);
        }
        if self.active {
            return None;
        }
        self.active = true;
        let double_tapped = self
            .last_release_ms
            .take()
            .is_some_and(|released| timestamp_ms.wrapping_sub(released) < DOUBLE_TAP_WINDOW_MS);
        self.second_tap = self.toggle || (double_tap_enabled && double_tapped);
        if double_tap_only && !self.second_tap {
            // Double-tap-only: the first tap arms the window silently;
            // recording waits for the second tap.
            self.recording = false;
            return None;
        }
        self.recording = true;
        Some(HotkeyAction::Start)
    }

    pub(super) fn release(
        &mut self,
        double_tap_enabled: bool,
        timestamp_ms: u32,
    ) -> Option<HotkeyAction> {
        if !self.active {
            return None;
        }
        self.active = false;
        if self.second_tap {
            self.second_tap = false;
            self.locked = true;
            self.last_release_ms = None;
            return None;
        }
        self.last_release_ms = double_tap_enabled.then_some(timestamp_ms);
        if !self.recording {
            // Releasing an armed-but-silent first tap emits nothing.
            return None;
        }
        self.recording = false;
        Some(HotkeyAction::Finish)
    }

    /// Another key joined a held modifier-only shortcut: it was a chord
    /// meant for another application. Only a recording capture reports
    /// it, since a bare cancel also cancels the newest unfinished job.
    pub(super) fn interrupt(&mut self) -> Option<HotkeyAction> {
        if !self.active {
            return None;
        }
        let recording = self.recording;
        self.active = false;
        self.second_tap = false;
        self.recording = false;
        self.last_release_ms = None;
        recording.then_some(HotkeyAction::Cancel)
    }

    /// Whether Toggle's cancel shortcut is Hex's now: only while a toggled
    /// capture records, so the key stays with every app otherwise.
    pub(super) fn wants_cancel(&self) -> bool {
        self.toggle && (self.active || self.locked)
    }

    pub(super) fn cancel(&mut self) -> Option<HotkeyAction> {
        if !self.active && !self.locked {
            return None;
        }
        self.active = false;
        self.locked = false;
        self.second_tap = false;
        self.recording = false;
        self.last_release_ms = None;
        Some(HotkeyAction::Cancel)
    }
}
