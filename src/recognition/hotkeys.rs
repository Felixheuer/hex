//! The dictation shortcut machines: dictation, Second dictation
//! shortcut while it is on and bound, Dictate without processing while it is
//! bound, Caps Lock dictation's hands-free toggle, More
//! dictation shortcuts' rows while that Lab is on, and the assistant's
//! Action shortcut while the assistant is on and it is bound.

use crate::app_settings::MAX_EXTRA_DICTATION_SHORTCUTS;
use crate::audio::CaptureInstant;
use crate::control::{PrintReply, PrintedTranscript};
use crate::suppression::{DictationHotkey, HotkeyAction, InputEvent};

/// One of the dictation shortcut machines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Shortcut {
    Dictation,
    /// Second dictation shortcut: dictates as the dictation shortcut
    /// does.
    Second,
    /// Dictate without processing.
    Raw,
    /// Caps Lock dictation: a press toggles a hands-free capture.
    CapsLock,
    /// More dictation shortcuts, by row.
    Extra(usize),
    /// The assistant's Action shortcut: records as dictation does, but
    /// its transcript asks the assistant; a tap then a hold starts a
    /// Background task, and a double tap shows the task list
    /// (`recognition::ask`).
    Action,
}

impl Shortcut {
    /// Every machine in matching order: the fixed shortcuts first, then the
    /// extra dictation rows.
    fn all() -> impl Iterator<Item = Self> {
        const FIXED: [Shortcut; 5] = [
            Shortcut::Dictation,
            Shortcut::Second,
            Shortcut::Raw,
            Shortcut::CapsLock,
            Shortcut::Action,
        ];
        FIXED
            .into_iter()
            .chain((0..MAX_EXTRA_DICTATION_SHORTCUTS).map(Shortcut::Extra))
    }
}

/// The dictation shortcut machines: dictation, Second dictation
/// shortcut while it is on and bound, Dictate without processing while it is
/// bound, More dictation shortcuts' rows, and Caps Lock
/// dictation's machine. One records at a time, and the one that started a
/// capture finishes or cancels it.
pub(super) struct Hotkeys {
    pub(super) dictation: DictationHotkey,
    pub(super) second: Option<DictationHotkey>,
    pub(super) raw: Option<DictationHotkey>,
    /// More dictation shortcuts: each bound row's own machine, with
    /// its own activation mode.
    pub(super) extra: [Option<DictationHotkey>; MAX_EXTRA_DICTATION_SHORTCUTS],
    /// Caps Lock dictation's hands-free toggle. Its binding is empty:
    /// presses arrive from the caps-lock worker, not the shortcut matcher.
    pub(super) caps_lock: DictationHotkey,
    /// The assistant's Action shortcut: Hold, with a double tap as a
    /// gesture rather than a lock, whatever the dictation shortcut's mode.
    pub(super) action: Option<DictationHotkey>,
    /// The recording (or the last) skips processing: the raw shortcut or
    /// `hex dictation start --raw` started it.
    pub(super) raw_capture: bool,
    /// The recording answers a `hex dictation start --print` instead of
    /// pasting or copying, however it ends.
    pub(super) print_capture: Option<PrintReply>,
    /// Whether the listener last saw shortcuts suspended, so that a capture
    /// a command starts during a pause is left alone.
    pub(super) suspended: bool,
}

impl Hotkeys {
    /// A listener's machines: the dictation shortcut and the Caps Lock
    /// machine, with no optional shortcut bound until `configure`.
    pub(super) fn new(dictation: DictationHotkey, caps_lock: DictationHotkey) -> Self {
        Self {
            dictation,
            second: None,
            raw: None,
            extra: std::array::from_fn(|_| None),
            caps_lock,
            action: None,
            raw_capture: false,
            print_capture: None,
            suspended: false,
        }
    }

    /// The fixed shortcuts' machines, in matching order. The extra rows have
    /// their own trigger modes and are configured separately.
    fn fixed_machines(&mut self) -> impl Iterator<Item = &mut DictationHotkey> {
        std::iter::once(&mut self.dictation)
            .chain(self.second.as_mut())
            .chain(self.raw.as_mut())
            .chain(Some(&mut self.caps_lock))
    }

    fn machines(&mut self) -> impl Iterator<Item = &mut DictationHotkey> {
        std::iter::once(&mut self.dictation)
            .chain(self.second.as_mut())
            .chain(self.raw.as_mut())
            .chain(Some(&mut self.caps_lock))
            .chain(self.extra.iter_mut().flatten())
            .chain(self.action.as_mut())
    }

    fn machine(&self, shortcut: Shortcut) -> Option<&DictationHotkey> {
        match shortcut {
            Shortcut::Dictation => Some(&self.dictation),
            Shortcut::Second => self.second.as_ref(),
            Shortcut::Raw => self.raw.as_ref(),
            Shortcut::CapsLock => Some(&self.caps_lock),
            Shortcut::Extra(index) => self.extra.get(index).and_then(|machine| machine.as_ref()),
            Shortcut::Action => self.action.as_ref(),
        }
    }

    fn machine_mut(&mut self, shortcut: Shortcut) -> Option<&mut DictationHotkey> {
        match shortcut {
            Shortcut::Dictation => Some(&mut self.dictation),
            Shortcut::Second => self.second.as_mut(),
            Shortcut::Raw => self.raw.as_mut(),
            Shortcut::CapsLock => Some(&mut self.caps_lock),
            Shortcut::Extra(index) => self
                .extra
                .get_mut(index)
                .and_then(|machine| machine.as_mut()),
            Shortcut::Action => self.action.as_mut(),
        }
    }

    /// Whether a machine other than `shortcut`'s records.
    fn recording_besides(&self, shortcut: Shortcut) -> bool {
        Shortcut::all()
            .filter(|other| *other != shortcut)
            .filter_map(|other| self.machine(other))
            .any(DictationHotkey::is_recording)
    }

    /// Whether a machine other than `shortcut`'s has a double-tap gesture
    /// waiting to become a recording.
    fn pending_gesture_besides(&self, shortcut: Shortcut) -> bool {
        Shortcut::all()
            .filter(|other| *other != shortcut)
            .filter_map(|other| self.machine(other))
            .any(DictationHotkey::has_pending_gesture)
    }

    pub(super) fn any_recording(&self) -> bool {
        Shortcut::all()
            .filter_map(|shortcut| self.machine(shortcut))
            .any(DictationHotkey::is_recording)
    }

    /// Whether the assistant's Action shortcut records a question.
    pub(super) fn action_recording(&self) -> bool {
        self.action
            .as_ref()
            .is_some_and(DictationHotkey::is_recording)
    }

    /// One of the Action machine's actions as the assistant takes it: a
    /// start that followed a tap is a Background task once held.
    pub(super) fn action_press(&self, action: HotkeyAction) -> super::ask::Press {
        super::ask::Press {
            action,
            after_tap: action == HotkeyAction::Start
                && self
                    .action
                    .as_ref()
                    .is_some_and(DictationHotkey::follows_tap),
        }
    }

    /// Whether one of the More dictation shortcuts records.
    pub(super) fn extra_recording(&self) -> bool {
        self.extra
            .iter()
            .flatten()
            .any(DictationHotkey::is_recording)
    }

    pub(super) fn suppresses_recognition(&self) -> bool {
        Shortcut::all()
            .filter_map(|shortcut| self.machine(shortcut))
            .any(DictationHotkey::suppresses_recognition)
    }

    /// Stands `shortcut`'s machine down, as when its start was refused.
    pub(super) fn suspend(&mut self, shortcut: Shortcut) {
        match self.machine_mut(shortcut) {
            Some(machine) => machine.suspend(),
            None => self.dictation.suspend(),
        };
    }

    /// Suspends every machine (never stopping at the first, as `any`
    /// would) and reports whether one was recording. A command still waiting
    /// for a print-only recording is answered: that recording is over.
    pub(super) fn suspend_all(&mut self) -> bool {
        self.fail_print("its recording already ended");
        self.machines().filter_map(DictationHotkey::suspend).count() > 0
    }

    /// Suspends every machine when shortcuts become suspended, once per
    /// suspension: the capture under way is cancelled, while a capture a
    /// command starts during the same suspension keeps recording. Reports
    /// whether one was recording when the suspension began.
    pub(super) fn suspend_when(&mut self, suspended: bool) -> bool {
        if std::mem::replace(&mut self.suspended, suspended) == suspended {
            return false;
        }
        suspended && self.suspend_all()
    }

    pub(super) fn disarm_pending_gestures(&mut self) {
        self.machines()
            .for_each(DictationHotkey::disarm_pending_gesture);
    }

    pub(super) fn track_key_state(&mut self, event: InputEvent, at: CaptureInstant) {
        for machine in self.machines() {
            machine.track_key_state(event, at);
        }
    }

    pub(super) fn recover_stale_keys(&mut self) {
        self.machines()
            .for_each(DictationHotkey::recover_stale_keys);
    }

    pub(super) fn break_chords(&mut self, now: CaptureInstant) {
        for machine in self.machines() {
            machine.break_chord(now);
        }
    }

    /// Applies the saved shortcut options. Unbinding the raw or second
    /// shortcut (or turning the second's setting off) while it records
    /// reports true: the caller cancels that recording. The same goes for an
    /// extra shortcut whose row disappears.
    pub(super) fn configure(&mut self, now: CaptureInstant) -> bool {
        let hotkeys = crate::app_settings::runtime_hotkeys();
        let raw_cancelled = rebind(&mut self.raw, hotkeys.raw_dictation, now);
        let second_cancelled = rebind(&mut self.second, hotkeys.second_dictation, now);
        let extra_cancelled = self.rebind_extra(hotkeys.extra_dictation, now);
        let action_cancelled = rebind(&mut self.action, hotkeys.action, now);
        self.dictation.set_binding(hotkeys.dictation);
        for machine in self.fixed_machines() {
            machine.set_double_tap_enabled(crate::app_settings::double_tap_lock());
            machine.set_double_tap_only(crate::app_settings::double_tap_only());
            machine.set_toggle(crate::app_settings::shortcut_toggle());
            machine.set_cancel(hotkeys.cancel);
        }
        // Caps Lock is hands-free by definition: its own machine always
        // treats a press as Toggle, and Escape cancels it in every mode.
        self.caps_lock
            .set_binding(crate::app_settings::RuntimeHotkey::default());
        self.caps_lock.set_double_tap_enabled(false);
        self.caps_lock.set_double_tap_only(false);
        self.caps_lock.set_toggle(true);
        self.caps_lock.set_cancel(Some(
            crate::app_settings::HotkeyBinding::cancel_default().runtime(),
        ));
        // Each extra row triggers its own way and keeps its own cancel key:
        // the configured one if Toggle mode arms it, else Escape.
        let extra_cancel = hotkeys
            .cancel
            .unwrap_or_else(|| crate::app_settings::HotkeyBinding::cancel_default().runtime());
        for (index, machine) in self.extra.iter_mut().enumerate() {
            let Some(machine) = machine else {
                continue;
            };
            machine.set_trigger(hotkeys.extra_dictation_modes[index]);
            machine.set_cancel(Some(extra_cancel));
            machine.set_cancel_in_any_mode(true);
        }
        if let Some(action) = &mut self.action {
            configure_action(action);
        }
        raw_cancelled || second_cancelled || extra_cancelled || action_cancelled
    }

    /// Rebinds the extra rows by binding identity, so clearing or adding one
    /// row keeps every other row's machine, including a capture in progress.
    /// Returns whether a dropped machine was recording.
    pub(super) fn rebind_extra(
        &mut self,
        bindings: [Option<crate::app_settings::RuntimeHotkey>; MAX_EXTRA_DICTATION_SHORTCUTS],
        now: CaptureInstant,
    ) -> bool {
        let mut existing = std::mem::take(&mut self.extra);
        let mut rebound: [Option<DictationHotkey>; MAX_EXTRA_DICTATION_SHORTCUTS] =
            std::array::from_fn(|_| None);
        for (index, binding) in bindings.into_iter().enumerate() {
            let Some(binding) = binding else {
                continue;
            };
            let matched = existing
                .iter()
                .position(|machine| machine.as_ref().is_some_and(|m| m.binding() == binding));
            rebound[index] = Some(match matched {
                Some(position) => existing[position].take().expect("matched a machine"),
                None => DictationHotkey::new(now, crate::app_settings::double_tap_lock(), binding),
            });
        }
        let cancelled = existing.iter().flatten().any(DictationHotkey::is_recording);
        self.extra = rebound;
        cancelled
    }

    /// A Caps Lock press, if the lab's machine should act on it. A hold
    /// capture, another shortcut recording, or a pending double-tap gesture
    /// is left undisturbed.
    pub(super) fn caps_lock_press(&mut self) -> Option<HotkeyAction> {
        if self.recording_besides(Shortcut::CapsLock)
            || self.pending_gesture_besides(Shortcut::CapsLock)
        {
            return None;
        }
        let action = self.caps_lock.caps_lock_toggle()?;
        if action == HotkeyAction::Start {
            self.raw_capture = false;
        }
        Some(action)
    }

    /// Stands a Caps Lock capture down when its lab turns off. Returns
    /// whether one was recording so the caller can cancel it.
    pub(super) fn stop_caps_lock(&mut self) -> bool {
        if self.caps_lock.is_recording() {
            self.caps_lock.suspend();
            true
        } else {
            false
        }
    }

    /// Keeps a `hex dictation start --print`'s reply for the recording it
    /// just started, which answers it instead of pasting or copying.
    pub(super) fn keep_print(&mut self, print: PrintReply) {
        self.print_capture = Some(print);
    }

    /// Takes the print-only recording's reply for the shortcut `action` that
    /// ends it. Only the dictation machine owns the reply: `hex dictation
    /// start` locks that one, and another shortcut's action leaves it alone.
    /// A finish hands the reply to transcription; a discard never spoke, and
    /// anything else Hex or the person ends it with is refused. A start means
    /// the recording it belonged to already ended, so it is failed rather
    /// than left to answer a later dictation.
    pub(super) fn print_for(
        &mut self,
        shortcut: Shortcut,
        action: HotkeyAction,
    ) -> Option<PrintReply> {
        if shortcut != Shortcut::Dictation {
            return None;
        }
        let print = self.print_capture.take()?;
        match action {
            HotkeyAction::Finish => return Some(print),
            HotkeyAction::Discard(_) | HotkeyAction::DoubleTap => {
                print.send(PrintedTranscript::NoSpeech);
            }
            HotkeyAction::Start => {
                print.send(PrintedTranscript::Failed(
                    "its recording already ended".into(),
                ));
            }
            HotkeyAction::PasteLast
            | HotkeyAction::PasteMeeting
            | HotkeyAction::Cancel
            | HotkeyAction::CancelShortcut => print.send(PrintedTranscript::Cancelled),
        }
        None
    }

    /// Answers a print-only recording that Hex itself ends without a
    /// transcript (a microphone loss, a suspended shortcut, a closed
    /// listener), so its command never waits for the full cap.
    pub(super) fn fail_print(&mut self, why: &str) {
        if let Some(print) = self.print_capture.take() {
            print.send(PrintedTranscript::Failed(why.into()));
        }
    }

    /// Answers a print-only recording that `hex dictation cancel` discards.
    pub(super) fn cancel_print(&mut self) {
        if let Some(print) = self.print_capture.take() {
            print.send(PrintedTranscript::Cancelled);
        }
    }

    /// Feeds one input to every machine, and returns what each did with
    /// which machine it was. A machine that starts while another records
    /// stands down until its keys come up, and only the dictation machine
    /// pastes.
    pub(super) fn process(
        &mut self,
        event: InputEvent,
        at: CaptureInstant,
    ) -> Vec<(Shortcut, HotkeyAction)> {
        let mut actions = Vec::new();
        for shortcut in Shortcut::all() {
            let others_recording = self.recording_besides(shortcut);
            let Some(machine) = self.machine_mut(shortcut) else {
                continue;
            };
            match machine.process(event, at) {
                Some(HotkeyAction::Start) if others_recording => {
                    machine.suspend();
                }
                Some(HotkeyAction::PasteLast | HotkeyAction::PasteMeeting)
                    if shortcut != Shortcut::Dictation => {}
                Some(action) => actions.push((shortcut, action)),
                None => {}
            }
        }
        actions
    }

    /// Stops whichever machine records, as a `stop` or `cancel` command does.
    /// A reply still waiting belongs to a recording that already ended, so it
    /// is answered rather than left to a later capture.
    pub(super) fn suspend_recording(&mut self) {
        self.fail_print("its recording already ended");
        for machine in self.machines() {
            if machine.is_recording() {
                machine.suspend();
            }
        }
    }
}

/// The Action shortcut holds to ask, taps then holds for a Background task,
/// and double-taps for the task list in every shortcut mode, whatever the
/// dictation shortcut's settings, and has no cancel key of its own: a double
/// tap is a gesture, never a lock.
pub(super) fn configure_action(action: &mut DictationHotkey) {
    action.set_double_tap_enabled(true);
    action.set_double_tap_only(false);
    action.set_toggle(false);
    action.set_cancel(None);
    action.set_double_tap_gesture(true);
}

/// Creates, rebinds, or removes an optional shortcut machine. Returns
/// whether a removed machine was recording.
pub(super) fn rebind(
    slot: &mut Option<DictationHotkey>,
    binding: Option<crate::app_settings::RuntimeHotkey>,
    now: CaptureInstant,
) -> bool {
    match binding {
        None => slot.take().is_some_and(|machine| machine.is_recording()),
        Some(binding) => {
            match slot.as_mut() {
                Some(machine) => machine.set_binding(binding),
                None => {
                    *slot = Some(DictationHotkey::new(
                        now,
                        crate::app_settings::double_tap_lock(),
                        binding,
                    ));
                }
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::app_settings::RuntimeHotkey;

    fn hotkeys() -> Hotkeys {
        let base = CaptureInstant::ZERO;
        let option = RuntimeHotkey {
            modifiers: crate::app_settings::HotkeyModifiers::option(),
            key_code: None,
        };
        Hotkeys::new(
            DictationHotkey::new(base, true, option),
            DictationHotkey::new(base, false, RuntimeHotkey::default()),
        )
    }

    /// Option dictates; right Command alone is the assistant's Action
    /// shortcut, configured as the listener configures it.
    fn action_hotkeys() -> Hotkeys {
        let mut hotkeys = hotkeys();
        let mut action = DictationHotkey::new(
            CaptureInstant::ZERO,
            false,
            crate::app_settings::HotkeyBinding::action_default().runtime(),
        );
        configure_action(&mut action);
        hotkeys.action = Some(action);
        hotkeys
    }

    const RIGHT_COMMAND: u64 =
        crate::app_settings::COMMAND_KEY_MASK | crate::app_settings::RIGHT_COMMAND_MASK;
    const ACTION: Shortcut = Shortcut::Action;

    /// Feeds right Command's modifier state at `ms` and returns the Action
    /// machine's actions as the assistant takes them (`action_press`).
    fn step(hotkeys: &mut Hotkeys, flags: u64, ms: u64) -> Vec<ask::Press> {
        let at = CaptureInstant::ZERO + Duration::from_secs(60) + Duration::from_millis(ms);
        hotkeys
            .process(InputEvent::Flags(flags), at)
            .into_iter()
            .map(|(shortcut, action)| {
                assert_eq!(shortcut, ACTION);
                hotkeys.action_press(action)
            })
            .collect()
    }

    fn start(after_tap: bool) -> ask::Press {
        ask::Press {
            action: HotkeyAction::Start,
            after_tap,
        }
    }

    fn just(action: HotkeyAction) -> ask::Press {
        ask::Press {
            action,
            after_tap: false,
        }
    }

    use super::super::ask;

    /// A hold records and its release finishes: the listener asks Here with
    /// what was said (`recognition::ask`). A second hold right after the
    /// first asks Here again; a hold is no tap, and nothing locks.
    #[test]
    fn a_hold_asks_here() {
        let mut hotkeys = action_hotkeys();
        let h = &mut hotkeys;
        assert_eq!(step(h, RIGHT_COMMAND, 0), [start(false)]);
        assert_eq!(step(h, 0, 1_500), [just(HotkeyAction::Finish)]);
        assert_eq!(step(h, RIGHT_COMMAND, 1_600), [start(false)]);
        assert_eq!(step(h, 0, 3_000), [just(HotkeyAction::Finish)]);
        assert!(!hotkeys.any_recording());
    }

    /// A tap, then within the double-tap window a press held past the hold
    /// threshold: the press starts after a tap (a Background task) and its
    /// release finishes it, as a hold's does. Nothing locks.
    #[test]
    fn a_tap_then_a_hold_asks_in_the_background() {
        let mut hotkeys = action_hotkeys();
        let h = &mut hotkeys;
        assert_eq!(step(h, RIGHT_COMMAND, 0), [start(false)]);
        assert_eq!(step(h, 0, 70), [just(HotkeyAction::Finish)]);
        assert_eq!(step(h, RIGHT_COMMAND, 200), [start(true)]);
        assert!(h.action_recording());
        assert_eq!(step(h, 0, 2_000), [just(HotkeyAction::Finish)]);
        assert!(!h.any_recording());
        // The next hold is Here again: the Background hold was no tap.
        assert_eq!(step(h, RIGHT_COMMAND, 2_100), [start(false)]);
        assert_eq!(step(h, 0, 3_500), [just(HotkeyAction::Finish)]);

        // A second press just past the window is a plain hold.
        assert_eq!(step(h, RIGHT_COMMAND, 6_000), [start(false)]);
        assert_eq!(step(h, 0, 6_070), [just(HotkeyAction::Finish)]);
        assert_eq!(step(h, RIGHT_COMMAND, 6_400), [start(false)]);
        assert_eq!(step(h, 0, 8_000), [just(HotkeyAction::Finish)]);
        assert!(!hotkeys.any_recording());
    }

    /// Two complete taps show the task list: the second tap's capture ends
    /// as a gesture, nothing locks, and the next press starts over. A second
    /// tap released after the first's window is still a tap, never a hold.
    #[test]
    fn a_double_tap_is_a_gesture() {
        let mut hotkeys = action_hotkeys();
        let h = &mut hotkeys;
        assert_eq!(step(h, RIGHT_COMMAND, 0), [start(false)]);
        assert_eq!(step(h, 0, 70), [just(HotkeyAction::Finish)]);
        assert_eq!(step(h, RIGHT_COMMAND, 150), [start(true)]);
        assert_eq!(step(h, 0, 220), [just(HotkeyAction::DoubleTap)]);
        // A third tap starts over rather than pairing with the gesture.
        assert_eq!(step(h, RIGHT_COMMAND, 300), [start(false)]);
        assert_eq!(step(h, 0, 360), [just(HotkeyAction::Finish)]);

        assert_eq!(step(h, RIGHT_COMMAND, 2_000), [start(false)]);
        assert_eq!(step(h, 0, 2_060), [just(HotkeyAction::Finish)]);
        assert_eq!(step(h, RIGHT_COMMAND, 2_300), [start(true)]);
        assert_eq!(step(h, 0, 2_420), [just(HotkeyAction::DoubleTap)]);
        assert!(!hotkeys.any_recording());
    }

    /// One tap alone only starts and finishes a capture too short to
    /// transcribe, so it asks nothing; a later press is a fresh one. A hold,
    /// then a quick tap: the hold asked, and the tap pairs with nothing.
    #[test]
    fn a_tap_alone_asks_nothing() {
        let mut hotkeys = action_hotkeys();
        let h = &mut hotkeys;
        assert_eq!(step(h, RIGHT_COMMAND, 0), [start(false)]);
        assert_eq!(step(h, 0, 80), [just(HotkeyAction::Finish)]);
        assert_eq!(step(h, RIGHT_COMMAND, 2_000), [start(false)]);
        assert_eq!(step(h, 0, 3_500), [just(HotkeyAction::Finish)]);

        assert_eq!(step(h, RIGHT_COMMAND, 6_000), [start(false)]);
        assert_eq!(step(h, 0, 7_000), [just(HotkeyAction::Finish)]);
        assert_eq!(step(h, RIGHT_COMMAND, 7_080), [start(false)]);
        assert_eq!(step(h, 0, 7_150), [just(HotkeyAction::Finish)]);
        assert!(!hotkeys.any_recording());
    }

    /// Right Command with another key is the app's ⌘ chord: the capture its
    /// press began is discarded, and left Command never starts one.
    #[test]
    fn right_command_with_a_key_is_a_normal_chord() {
        let at =
            |ms: u64| CaptureInstant::ZERO + Duration::from_secs(60) + Duration::from_millis(ms);
        let mut hotkeys = action_hotkeys();
        let key = |code: u16, down: bool, flags: u64| InputEvent::Key { code, down, flags };
        assert_eq!(
            hotkeys.process(InputEvent::Flags(RIGHT_COMMAND), at(0)),
            [(ACTION, HotkeyAction::Start)]
        );
        // C, as in ⌘C.
        assert_eq!(
            hotkeys.process(key(8, true, RIGHT_COMMAND), at(60)),
            [(
                ACTION,
                HotkeyAction::Discard(crate::suppression::DiscardCause::Key)
            )]
        );
        assert!(
            hotkeys
                .process(key(8, false, RIGHT_COMMAND), at(120))
                .is_empty()
        );
        assert!(hotkeys.process(InputEvent::Flags(0), at(200)).is_empty());
        assert!(!hotkeys.any_recording());

        let left_command =
            crate::app_settings::COMMAND_KEY_MASK | crate::app_settings::LEFT_COMMAND_MASK;
        assert!(
            hotkeys
                .process(InputEvent::Flags(left_command), at(1_000))
                .is_empty()
        );
        assert!(hotkeys.process(InputEvent::Flags(0), at(2_000)).is_empty());
    }

    /// The shortcut that starts a capture owns it: dictation and the Action
    /// shortcut each stand down while the other records.
    #[test]
    fn the_action_shortcut_and_dictation_never_share_a_capture() {
        let option = crate::app_settings::OPTION_KEY_MASK;
        let at =
            |ms: u64| CaptureInstant::ZERO + Duration::from_secs(60) + Duration::from_millis(ms);
        let mut hotkeys = action_hotkeys();
        let mut press = |flags: u64, ms: u64| hotkeys.process(InputEvent::Flags(flags), at(ms));
        assert_eq!(
            press(option, 0),
            [(Shortcut::Dictation, HotkeyAction::Start)]
        );
        assert!(press(option | RIGHT_COMMAND, 500).is_empty());
        assert_eq!(
            press(0, 1_000),
            [(Shortcut::Dictation, HotkeyAction::Finish)]
        );

        assert_eq!(press(RIGHT_COMMAND, 2_000), [(ACTION, HotkeyAction::Start)]);
        assert!(press(RIGHT_COMMAND | option, 2_500).is_empty());
        assert_eq!(press(0, 3_000), [(ACTION, HotkeyAction::Finish)]);
        assert!(!hotkeys.any_recording());
    }

    /// With the assistant off or the shortcut unbound there is no Action machine:
    /// right Command does nothing.
    #[test]
    fn without_its_machine_right_command_does_nothing() {
        let mut hotkeys = hotkeys();
        let at =
            |ms: u64| CaptureInstant::ZERO + Duration::from_secs(60) + Duration::from_millis(ms);
        assert!(
            hotkeys
                .process(InputEvent::Flags(RIGHT_COMMAND), at(0))
                .is_empty()
        );
        assert!(hotkeys.process(InputEvent::Flags(0), at(1_000)).is_empty());
        assert!(!hotkeys.action_recording());
    }

    #[test]
    fn a_pause_cancels_once_and_leaves_a_command_capture_alone() {
        let mut hotkeys = hotkeys();
        assert!(!hotkeys.suspend_when(false));

        // A capture under way is cancelled as the pause begins.
        assert!(hotkeys.dictation.lock());
        assert!(hotkeys.any_recording());
        assert!(hotkeys.suspend_when(true));
        assert!(!hotkeys.any_recording());

        // While the same pause holds, a command's capture keeps recording.
        assert!(hotkeys.dictation.lock());
        assert!(!hotkeys.suspend_when(true));
        assert!(hotkeys.any_recording());

        // Resuming leaves it alone too; the next pause cancels it.
        assert!(!hotkeys.suspend_when(false));
        assert!(hotkeys.any_recording());
        assert!(hotkeys.suspend_when(true));
        assert!(!hotkeys.any_recording());
    }

    /// A `hex dictation start --print` recording is finished by the shortcut
    /// itself, and its reply goes with the capture to transcription.
    #[test]
    fn a_printing_start_follows_the_shortcut_that_finishes_it() {
        let base = CaptureInstant::ZERO;
        let at = |ms: u64| base + Duration::from_millis(ms);
        let mut hotkeys = hotkeys();
        assert!(hotkeys.dictation.lock());
        let (start, printed) = PrintReply::channel();
        hotkeys.keep_print(start);

        // The next shortcut press finishes the locked recording.
        assert_eq!(
            hotkeys.process(
                InputEvent::Flags(crate::app_settings::OPTION_KEY_MASK),
                at(0)
            ),
            [(Shortcut::Dictation, HotkeyAction::Finish)]
        );
        // Only `print_for` takes the reply; the action alone leaves it.
        assert!(hotkeys.print_capture.is_some());
        let print = hotkeys
            .print_for(Shortcut::Dictation, HotkeyAction::Finish)
            .expect("the transcript goes to the printing start");
        print.send(PrintedTranscript::Text("Ship it.".into()));
        assert_eq!(
            printed.recv_timeout(Duration::from_millis(50)),
            Ok(PrintedTranscript::Text("Ship it.".into()))
        );

        // A cancel is refused rather than waited out.
        let (start, printed) = PrintReply::channel();
        hotkeys.keep_print(start);
        assert!(
            hotkeys
                .print_for(Shortcut::Dictation, HotkeyAction::Cancel)
                .is_none()
        );
        assert!(matches!(
            printed.recv_timeout(Duration::from_millis(50)),
            Ok(PrintedTranscript::Cancelled)
        ));
    }

    /// A held reply belongs to the recording that started it: another
    /// shortcut leaves it alone, and a later start or a suspension answers it
    /// instead of letting it wait for a new dictation.
    #[test]
    fn a_held_print_reply_never_answers_a_later_dictation() {
        use crate::suppression::DiscardCause;

        let mut hotkeys = hotkeys();

        // The raw shortcut's finish isn't the dictation machine's, so the
        // reply stays held for its own recording.
        let (start, printed) = PrintReply::channel();
        hotkeys.keep_print(start);
        assert!(
            hotkeys
                .print_for(Shortcut::Raw, HotkeyAction::Finish)
                .is_none()
        );
        assert!(hotkeys.print_capture.is_some());
        assert!(matches!(
            printed.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        // A start means that recording already ended: fail it, never let it
        // wait for the new capture.
        assert!(
            hotkeys
                .print_for(Shortcut::Dictation, HotkeyAction::Start)
                .is_none()
        );
        assert!(hotkeys.print_capture.is_none());
        assert!(matches!(
            printed.recv_timeout(Duration::from_millis(50)),
            Ok(PrintedTranscript::Failed(why)) if why == "its recording already ended"
        ));

        // A short tap or chord never spoke.
        let (start, printed) = PrintReply::channel();
        hotkeys.keep_print(start);
        assert!(
            hotkeys
                .print_for(
                    Shortcut::Dictation,
                    HotkeyAction::Discard(DiscardCause::Key)
                )
                .is_none()
        );
        assert!(matches!(
            printed.recv_timeout(Duration::from_millis(50)),
            Ok(PrintedTranscript::NoSpeech)
        ));

        // Suspending every machine answers a reply that was still waiting.
        let (start, printed) = PrintReply::channel();
        hotkeys.keep_print(start);
        hotkeys.suspend_all();
        assert!(hotkeys.print_capture.is_none());
        assert!(matches!(
            printed.recv_timeout(Duration::from_millis(50)),
            Ok(PrintedTranscript::Failed(_))
        ));
    }
}
