//! The assistant's shortcuts in the Linux shortcut machine: the Action
//! shortcut's gestures (`assistant::trigger::Gesture`) and Stop all tasks.
//!
//! The Action shortcut owns the capture it starts, as the other dictation
//! shortcuts do (`DictationFlavour::Assistant`): dictation never starts over
//! it, it never starts over dictation, and only its own release (never
//! `hex dictation stop`) asks its question. The desktop may repeat a held
//! shortcut's press (GNOME, KWin), so a press while it is down is a repeat;
//! `ShortcutState::expire` counts repeats that stop as its release, as it
//! does for dictation.

use std::time::Instant;

use super::{Capture, HotkeyEvent, KEY_REPEAT_GAP, ShortcutState};
use crate::assistant::trigger::{Gesture, GestureAction};
use crate::dictation::DictationFlavour;

/// What the machine keeps for the assistant's shortcuts.
#[derive(Debug, Default)]
pub(super) struct AssistantKeys {
    gesture: Gesture,
    /// When Stop all tasks last went down, while its release hasn't come:
    /// a press sooner than a new one could come is the desktop repeating it.
    stop_pressed: Option<Instant>,
}

impl AssistantKeys {
    /// Whatever the Action shortcut armed or held is forgotten.
    pub(super) fn cancel(&mut self) {
        self.gesture.cancel();
    }

    /// A press of Stop all tasks, unless it repeats the last.
    pub(super) fn stop_all(&mut self, now: Instant) -> Option<HotkeyEvent> {
        let repeat = self
            .stop_pressed
            .is_some_and(|pressed| now.saturating_duration_since(pressed) < KEY_REPEAT_GAP);
        self.stop_pressed = Some(now);
        (!repeat).then_some(HotkeyEvent::StopAll)
    }

    pub(super) fn stop_all_released(&mut self) {
        self.stop_pressed = None;
    }
}

impl ShortcutState {
    /// The Action shortcut went down at `now`.
    pub(super) fn action_pressed(&mut self, now: Instant) -> Option<HotkeyEvent> {
        if !self.claim(DictationFlavour::Assistant) {
            return None;
        }
        let repeat = self.down
            && self
                .last_press
                .is_some_and(|pressed| now.saturating_duration_since(pressed) < KEY_REPEAT_GAP);
        self.last_press = Some(now);
        self.down = true;
        self.repeating = repeat;
        if repeat {
            return None;
        }
        // The desktop never sent the release of the press before (as it
        // drops dictation's): that press finishes, and this one is lost.
        if self.assistant.gesture.is_down() {
            return self.action_released(now);
        }
        let Some(GestureAction::Start { after_tap }) = self.assistant.gesture.press(now) else {
            return None;
        };
        self.capture = Capture::Held { second_tap: false };
        self.held_since = Some(now);
        Some(HotkeyEvent::Ask { after_tap })
    }

    /// The Action shortcut came up at `now`.
    pub(super) fn action_released(&mut self, now: Instant) -> Option<HotkeyEvent> {
        if !self.flavour.assistant() {
            return None;
        }
        self.down = false;
        self.repeating = false;
        let gesture = self.assistant.gesture.release(now)?;
        self.capture = Capture::Idle;
        self.last_release = None;
        Some(match gesture {
            GestureAction::DoubleTap => HotkeyEvent::DoubleTap,
            _ => HotkeyEvent::Finish,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::{HotkeyOptions, Input, Shortcut, ShortcutCommand};
    use super::*;

    /// Feeds inputs at `ms` after a start and keeps what the machine said.
    struct Machine {
        state: ShortcutState,
        start: Instant,
    }

    impl Machine {
        fn new() -> Self {
            Self {
                state: ShortcutState::new(&HotkeyOptions {
                    double_tap: true,
                    ..HotkeyOptions::default()
                }),
                start: Instant::now(),
            }
        }

        fn at(&mut self, ms: u64, input: Input) -> Option<HotkeyEvent> {
            self.state
                .update(input, self.start + Duration::from_millis(ms))
        }
    }

    const ACTION: Shortcut = Shortcut::Action;

    /// A hold asks; a tap then a hold asks a Background task; two taps are
    /// the double tap, which leaves nothing armed.
    #[test]
    fn the_action_shortcut_tells_its_gestures_apart() {
        let mut m = Machine::new();
        let ask = |after_tap| Some(HotkeyEvent::Ask { after_tap });
        assert_eq!(m.at(0, Input::Pressed(ACTION)), ask(false));
        assert_eq!(
            m.at(1_500, Input::Released(ACTION)),
            Some(HotkeyEvent::Finish)
        );
        assert_eq!(m.at(2_000, Input::Pressed(ACTION)), ask(false));
        assert_eq!(
            m.at(2_070, Input::Released(ACTION)),
            Some(HotkeyEvent::Finish)
        );
        assert_eq!(m.at(2_200, Input::Pressed(ACTION)), ask(true));
        assert_eq!(
            m.at(4_000, Input::Released(ACTION)),
            Some(HotkeyEvent::Finish)
        );
        assert_eq!(m.at(6_000, Input::Pressed(ACTION)), ask(false));
        assert_eq!(
            m.at(6_060, Input::Released(ACTION)),
            Some(HotkeyEvent::Finish)
        );
        assert_eq!(m.at(6_200, Input::Pressed(ACTION)), ask(true));
        assert_eq!(
            m.at(6_280, Input::Released(ACTION)),
            Some(HotkeyEvent::DoubleTap)
        );
        assert_eq!(m.at(6_350, Input::Pressed(ACTION)), ask(false));
    }

    /// The Action shortcut owns its capture: dictation starts nothing over
    /// it, `hex dictation stop` never asks its question, and it starts
    /// nothing over a dictation.
    #[test]
    fn the_action_shortcut_and_dictation_never_share_a_capture() {
        let mut m = Machine::new();
        assert!(m.at(0, Input::Pressed(ACTION)).is_some());
        assert_eq!(m.at(100, Input::Pressed(Shortcut::Dictation)), None);
        assert_eq!(
            m.at(200, Input::Command(ShortcutCommand::DictationStop)),
            None
        );
        assert_eq!(m.at(300, Input::Released(Shortcut::Dictation)), None);
        assert_eq!(
            m.at(1_000, Input::Released(ACTION)),
            Some(HotkeyEvent::Finish)
        );

        assert!(matches!(
            m.at(2_000, Input::Pressed(Shortcut::Dictation)),
            Some(HotkeyEvent::Start { .. })
        ));
        assert_eq!(m.at(2_100, Input::Pressed(ACTION)), None);
        assert_eq!(m.at(2_200, Input::Released(ACTION)), None);
    }

    /// A desktop that repeats the held press repeats nothing here, and one
    /// that drops the release finishes the hold once its repeats stop.
    #[test]
    fn repeats_and_a_lost_release_finish_once() {
        let mut m = Machine::new();
        assert!(m.at(0, Input::Pressed(ACTION)).is_some());
        assert_eq!(m.at(500, Input::Pressed(ACTION)), None);
        assert_eq!(m.at(530, Input::Pressed(ACTION)), None);
        assert_eq!(
            m.state.expire(m.start + Duration::from_millis(2_000)),
            Some(HotkeyEvent::Finish)
        );
        assert!(!m.state.assistant.gesture.is_down());
    }

    /// Stop all tasks stops once per press, whatever records, and its
    /// repeats stop nothing.
    #[test]
    fn stop_all_presses_once() {
        let mut m = Machine::new();
        let stop = Some(HotkeyEvent::StopAll);
        assert_eq!(m.at(0, Input::Pressed(Shortcut::StopAll)), stop);
        assert_eq!(m.at(400, Input::Pressed(Shortcut::StopAll)), None);
        assert_eq!(m.at(450, Input::Released(Shortcut::StopAll)), None);
        assert_eq!(m.at(600, Input::Pressed(Shortcut::StopAll)), stop);
        assert!(m.at(700, Input::Pressed(ACTION)).is_some());
        assert_eq!(m.at(3_000, Input::Pressed(Shortcut::StopAll)), stop);
    }
}
