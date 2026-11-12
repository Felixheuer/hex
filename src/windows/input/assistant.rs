//! The assistant's keys in the keyboard hook: the Action shortcut's
//! gestures (`assistant::trigger::Gesture`), Stop all tasks, and Escape
//! while a Here task runs or an answer is read aloud.
//!
//! The Action shortcut owns the capture it starts, as the other dictation
//! shortcuts do (`DictationFlavour::Assistant`), so dictation never starts
//! over it and it never starts over dictation. Hex swallows Stop all
//! tasks while it is bound, and Escape only while the listener last found a
//! Here task running or speech playing (`trigger::escape_claimed`), so
//! Escape stays with the app in front otherwise.

use std::time::Instant;

use super::*;
use crate::assistant::trigger::{self, Gesture, GestureAction, StopAllKey};

/// No modifier held: Escape alone.
const BARE: WindowsHotkey = WindowsHotkey {
    control: None,
    windows: None,
    alt: None,
    shift: None,
    key: None,
};

/// The assistant's shortcuts as the hook matches them.
#[derive(Default)]
pub(super) struct AssistantKeys {
    action: Option<WindowsHotkey>,
    action_vk: Option<u32>,
    /// The Action key went down while Hex held it; its repeats and release
    /// are ours too.
    action_down: bool,
    gesture: Gesture,
    stop_all: Option<WindowsHotkey>,
    stop_all_vk: Option<u32>,
    stop_held: StopAllKey<u32>,
    /// Escape went down while the assistant held it; its release is ours.
    escape_down: bool,
}

impl AssistantKeys {
    /// The keys of `action` and `stop_all`. A key the hook can't match
    /// leaves that shortcut unbound, with a warning, so dictation still
    /// starts.
    pub(super) fn new(action: Option<WindowsHotkey>, stop_all: Option<WindowsHotkey>) -> Self {
        let key = |binding: Option<WindowsHotkey>, name: &str| match chord_virtual_key(
            binding.as_ref(),
            name,
        ) {
            Ok(vk) => (binding, vk),
            Err(error) => {
                tracing::warn!(%error, "the {name} can't be matched; it does nothing");
                (None, None)
            }
        };
        let (action, action_vk) = key(action, "Action shortcut");
        let (stop_all, stop_all_vk) = key(stop_all, "Stop all tasks shortcut");
        Self {
            action,
            action_vk,
            stop_all,
            stop_all_vk,
            ..Self::default()
        }
    }

    /// The Action shortcut's binding, whose modifiers warm the HUD.
    pub(super) fn action(&self) -> Option<&WindowsHotkey> {
        self.action.as_ref()
    }

    /// A chord recording started: whatever is down is let go.
    pub(super) fn reset(&mut self) {
        self.action_down = false;
        self.gesture.cancel();
        self.stop_held = StopAllKey::default();
        self.escape_down = false;
    }
}

impl CallbackState {
    /// The assistant's turn at a key: `Some` when it took the key.
    pub(super) fn assistant_key(
        &mut self,
        vk: u32,
        is_down: bool,
        time: u32,
    ) -> Option<Disposition> {
        if self.assistant.stop_all_vk == Some(vk) {
            let matches = self
                .assistant
                .stop_all
                .as_ref()
                .is_some_and(|binding| self.modifiers.matches(binding));
            if self.assistant.stop_held.is_down() || (is_down && matches) {
                self.mask_release |= is_down && holds_menu(self.assistant.stop_all.as_ref());
                if self.assistant.stop_held.press(vk, is_down, matches) {
                    send_event(self, HotkeyAction::StopAll, time);
                }
                return Some(Disposition::Suppress);
            }
        }
        if self.assistant.action_vk == Some(vk) {
            let triggered = self.assistant.action_down
                || (is_down
                    && !self.dirty
                    && !self.capture.active
                    && !self.capture.locked
                    && self
                        .assistant
                        .action
                        .as_ref()
                        .is_some_and(|binding| self.modifiers.matches(binding)));
            if triggered {
                self.action(is_down, time);
                return Some(Disposition::Suppress);
            }
        }
        // Escape stops the Here task or the answer read aloud, only while
        // no dictation records (it never ends one, #786).
        if vk == u32::from(VK_ESCAPE)
            && (self.assistant.escape_down
                || (is_down
                    && !self.capture.active
                    && !self.capture.locked
                    && self.modifiers.matches(&BARE)
                    && trigger::escape_claimed()))
        {
            if is_down && !self.assistant.escape_down {
                send_event(self, HotkeyAction::StopHere, time);
            }
            self.assistant.escape_down = is_down;
            return Some(Disposition::Suppress);
        }
        None
    }

    /// A press or release of the Action shortcut's key.
    fn action(&mut self, is_down: bool, time: u32) {
        self.mask_release |= is_down && holds_menu(self.assistant.action.as_ref());
        if is_down {
            if std::mem::replace(&mut self.assistant.action_down, true) {
                return;
            }
            if let Some(GestureAction::Start { after_tap }) =
                self.assistant.gesture.press(Instant::now())
            {
                // The capture is the Action shortcut's until it comes up.
                self.capture.claim(DictationFlavour::Assistant);
                self.capture.active = true;
                self.capture.recording = true;
                send_event(self, HotkeyAction::Ask { after_tap }, time);
            }
            return;
        }
        self.assistant.action_down = false;
        let Some(gesture) = self.assistant.gesture.release(Instant::now()) else {
            return;
        };
        if !self.capture.active || self.capture.flavour != DictationFlavour::Assistant {
            // A chord recording took the capture meanwhile.
            return;
        }
        self.capture.active = false;
        self.capture.recording = false;
        self.capture.last_release_ms = None;
        let action = match gesture {
            GestureAction::DoubleTap => HotkeyAction::DoubleTap,
            _ => HotkeyAction::Finish,
        };
        send_event(self, action, time);
    }
}
