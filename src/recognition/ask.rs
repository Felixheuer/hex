//! The assistant's Action shortcut and Stop all tasks, from the
//! listener's side (macOS).
//!
//! The Action machine (`Shortcut::Action`) records exactly as dictation
//! does: the same microphone, HUD, sounds, chord discards, and short-tap
//! discards. Its gestures:
//!
//! - **Hold** is for everything: say anything, and the assistant decides
//!   whether to answer, act in the window in front, or escalate to a
//!   background job. It always asks as `Origin::ActionShortcut`, continuing
//!   `assistant::focus` (a task waiting on a question first, then a running
//!   Here task, then a recent one), so a hold answers an open question or
//!   steers a running task. The desktop is read on its own thread once the
//!   hold is real (past the hold threshold with no other key), so a ⌘C chord
//!   reads nothing; the screenshot leaves out Hex's own windows.
//! - **Tap, then hold** (the second press within the double-tap window, held
//!   past the hold threshold) is an optional shortcut that forces a new
//!   Background task (`Origin::Background`) with no conversation and no
//!   screenshot. It never pastes.
//! - **Double tap** (two taps, no hold) shows or hides the HUD's task list
//!   (`assistant::trigger::set_toggle_tasks`).
//! - A Here question that starts with "im Hintergrund", "in the background",
//!   "nebenbei", "parallel", or "Hintergrund:" goes to the Background lane
//!   instead, without those words (`assistant::trigger::spoken_background`).
//!
//! A hold that becomes real prewarms its lane (`assistant::service::prewarm`).
//! What a press means and what its words ask is shared with Windows and
//! Linux (`assistant::trigger`); this carries it out with the macOS
//! listener's capture. The release hands the clip to transcription as
//! `TranscriptionTarget::Assistant`, processed like a paste but never pasted,
//! copied, kept for Paste last dictation, or saved to History; its own thread
//! then submits the words. Silence, a discard, or a failed transcription asks
//! nothing, and only words for the Here lane stop the answer being read
//! aloud.
//!
//! Escape stops a running Here task and its speech, never a Background one,
//! and Hex takes Escape from the app in front only while one runs or speaks.
//! The Stop all tasks shortcut (`assistant.stop_all_hotkey`, ⌃⌥Esc) stops
//! every task and the speech; the event tap swallows it only while the
//! assistant is on and it is bound (`RuntimeHotkeys::stop_all`).

use color_eyre::Result;

use crate::app_settings::RuntimeHotkey;
use crate::assistant::trigger::{self, Asking, Start, StopAllKey};
use crate::audio::CaptureInstant;
use crate::control::{ControlReply, PrintReply};
use crate::desktop::events::EventLog;
use crate::dictation::POST_RELEASE_TAIL;
use crate::dictation::audio::DictationAudio;
use crate::dictation::context::ContextSnapshot;
use crate::feedback::{self, Tone};
use crate::parakeet::DictationWorker;
use crate::suppression::{HotkeyAction, InputEvent};

use super::context_watch::ContextWatch;
use super::{DictationSink, FinishResult};

/// One of the Action machine's actions, and whether the press it starts
/// followed a tap (`Hotkeys::action_press`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Press {
    pub(super) action: HotkeyAction,
    /// A tap came just before: held, this press is a Background task.
    pub(super) after_tap: bool,
}

/// The listener's Action shortcut: the question its press settled, whether
/// the tap keeps Escape for the assistant now, and the Stop all key held.
#[derive(Default)]
pub(super) struct Asker {
    /// The question the press under way asks (`assistant::trigger`).
    asking: Asking,
    escape: bool,
    /// The Stop all key's code while it is down, so its repeats stop nothing
    /// twice.
    stop_held: StopAllKey<u16>,
    /// Conversation mode and barge-in.
    talk: super::talk::Talk,
}

impl Asker {
    /// Carries out one of the Action machine's actions. Returns whether a
    /// start was accepted, as `handle_hotkey_action` does.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn act(
        &mut self,
        press: Press,
        at: CaptureInstant,
        input: &DictationAudio,
        worker: &DictationWorker,
        context: &mut ContextWatch,
        device: &str,
        events: &mut EventLog,
        indicator: DictationSink<'_>,
        release_while_idle: bool,
    ) -> Result<bool> {
        let handle = |action, context: &mut ContextWatch, events: &mut EventLog| {
            super::handle_hotkey_action(
                action,
                false,
                None,
                at,
                input,
                worker,
                context,
                device,
                events,
                indicator,
                release_while_idle,
            )
        };
        let action = press.action;
        if action == HotkeyAction::Start {
            // Not set up yet: the press records nothing, and a hold (never
            // a tap or a ⌘C chord) opens the setup as it comes up. Else it
            // reads nothing yet: a chord (⌘C) or a short tap is discarded
            // before the hold is real (`intend`).
            if self.asking.start(press.after_tap, at) == Start::Setup {
                return Ok(true);
            }
            let accepted = handle(action, context, events)?;
            if accepted {
                // The HUD wears the assistant's colours (and after a tap
                // says "New task"), so a question never looks like a
                // dictation.
                indicator.send(
                    crate::dictation::indicator::DictationIndicatorEvent::Asking {
                        agent: press.after_tap,
                    },
                );
            } else {
                self.asking.drop_question();
            }
            return Ok(accepted);
        }
        if self.asking.setting_up() {
            if action == HotkeyAction::Finish {
                self.asking.finish(at);
            } else {
                self.asking.drop_question();
            }
            return Ok(true);
        }
        match action {
            HotkeyAction::Finish => {
                let page = context.finish(events)?;
                let Some(reply) = self.asking.finish(at) else {
                    return Ok(true);
                };
                finish_question(
                    at,
                    input,
                    worker,
                    &page,
                    reply,
                    device,
                    events,
                    indicator,
                    release_while_idle,
                )?;
                Ok(true)
            }
            // The second tap's capture ends unheard, as a short tap's does,
            // and the task list shows or hides.
            HotkeyAction::DoubleTap => {
                handle(action, context, events)?;
                self.asking.double_tap();
                Ok(true)
            }
            _ => {
                self.asking.drop_question();
                handle(action, context, events)
            }
        }
    }

    /// The press became real (held past the hold threshold with no other
    /// key): a Here question starts reading the desktop now, and its lane
    /// gets ready to answer.
    pub(super) fn intend(&mut self) {
        self.asking.intend();
    }

    /// Whether the tap keeps Escape from the app in front for the assistant
    /// now, remembered for the Escape that arrives next.
    pub(super) fn arm_escape(&mut self, bound: bool, free: bool) -> bool {
        self.escape = super::guards::assistant_claims_escape(bound, free, trigger::claims_escape);
        self.escape
    }

    /// A key for the assistant: any key ends conversation mode (closing its
    /// capture before a shortcut can start another); the Stop all tasks
    /// shortcut stops every task and the speech; Escape while the assistant
    /// took it stops the Here tasks and the speech, never a Background task
    /// (each has its own stop).
    pub(super) fn key(
        &mut self,
        event: InputEvent,
        input: &DictationAudio,
        indicator: DictationSink<'_>,
    ) {
        self.talk.key(event, input, indicator);
        let stop_all = crate::app_settings::runtime_hotkeys().stop_all;
        if stop_all_press(event, stop_all, &mut self.stop_held) {
            trigger::stop_all();
            return;
        }
        if self.escape && event.is_escape_down() {
            trigger::stop_here();
        }
    }

    /// Microphone audio for conversation mode, which hears only its own
    /// capture.
    pub(super) fn hear(&mut self, samples: &[f32], rate: u32, indicator: DictationSink<'_>) {
        self.talk.hear(samples, rate, indicator);
    }

    /// Conversation mode's turn in the listener's loop: after a Here answer
    /// it listens for a follow-up, and with barge-in hears one over the
    /// answer (`recognition::talk`). `blocked`: another capture, a command's
    /// dictation, or a pause owns the microphone.
    pub(super) fn converse(
        &mut self,
        blocked: bool,
        input: &DictationAudio,
        worker: &DictationWorker,
        events: &mut EventLog,
        indicator: DictationSink<'_>,
        release_while_idle: bool,
    ) -> Result<()> {
        self.talk.converse(
            blocked,
            input,
            worker,
            events,
            indicator,
            release_while_idle,
        )
    }

    /// Whether conversation mode's capture is open (a microphone loss then
    /// never salvages it as a dictation).
    pub(super) fn conversing(&self) -> bool {
        self.talk.conversing()
    }
}

/// Whether `event` is a press of the Stop all tasks shortcut `binding`
/// (`None` with the assistant off or it unbound). `held` keeps its key while it is
/// down, so the key's repeats are not presses.
fn stop_all_press(
    event: InputEvent,
    binding: Option<RuntimeHotkey>,
    held: &mut StopAllKey<u16>,
) -> bool {
    let InputEvent::Key { code, down, flags } = event else {
        return false;
    };
    held.press(
        code,
        down,
        binding.is_some_and(|binding| binding.matches_key_press(code, flags)),
    )
}

/// A `hex dictation stop` or `toggle` leaves a question being asked alone:
/// only the Action shortcut's release asks it.
pub(super) fn busy_reply() -> ControlReply {
    ControlReply::error(
        "busy",
        "the Action shortcut is asking the assistant; release it, or cancel with \
         `hex dictation cancel`",
    )
}

/// Ends the question's capture at `at` and hands its clip to transcription
/// for `reply`, as a shortcut dictation's finish does for a paste.
#[allow(clippy::too_many_arguments)]
pub(super) fn finish_question(
    at: CaptureInstant,
    input: &DictationAudio,
    worker: &DictationWorker,
    page: &ContextSnapshot,
    reply: PrintReply,
    device: &str,
    events: &mut EventLog,
    indicator: DictationSink<'_>,
    release_while_idle: bool,
) -> Result<()> {
    if input.become_intentional(at)? && release_while_idle {
        feedback::play(Tone::DictationStart);
    }
    // A discarded or silent clip drops the reply unanswered: nothing is asked.
    let finished = super::finish_dictation_with(
        input,
        at,
        POST_RELEASE_TAIL,
        |clip, _| worker.transcribe_for_assistant(clip, reply, page.clone()),
        device,
        events,
        indicator,
    )?;
    if matches!(finished, FinishResult::Discarded) {
        feedback::retract_start();
    }
    super::emit_engine_state(events, false, worker, device)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESCAPE: u16 = 53;

    fn key(code: u16, down: bool, flags: u64) -> InputEvent {
        InputEvent::Key { code, down, flags }
    }

    /// ⌃⌥Esc stops everything once per press, its repeats stop nothing, and
    /// Escape alone, ⌘⌥Esc (Force Quit), or the shortcut unbound or the
    /// assistant off (`None`) is no press.
    #[test]
    fn the_stop_all_shortcut_presses_once_and_only_while_bound() {
        use crate::app_settings::{COMMAND_KEY_MASK, CONTROL_KEY_MASK, OPTION_KEY_MASK};
        let binding = Some(crate::app_settings::HotkeyBinding::stop_all_default().runtime());
        let chord = CONTROL_KEY_MASK | OPTION_KEY_MASK;
        let mut held = StopAllKey::default();
        assert!(stop_all_press(key(ESCAPE, true, chord), binding, &mut held));
        assert!(!stop_all_press(
            key(ESCAPE, true, chord),
            binding,
            &mut held
        ));
        assert!(!stop_all_press(
            key(ESCAPE, false, chord),
            binding,
            &mut held
        ));
        assert!(stop_all_press(key(ESCAPE, true, chord), binding, &mut held));
        assert!(!stop_all_press(key(ESCAPE, false, 0), binding, &mut held));

        assert!(!stop_all_press(key(ESCAPE, true, 0), binding, &mut held));
        assert!(!stop_all_press(key(ESCAPE, false, 0), binding, &mut held));
        let force_quit = COMMAND_KEY_MASK | OPTION_KEY_MASK;
        assert!(!stop_all_press(
            key(ESCAPE, true, force_quit),
            binding,
            &mut held
        ));
        assert!(!stop_all_press(key(12, true, chord), binding, &mut held));
        assert!(!stop_all_press(
            InputEvent::Flags(chord),
            binding,
            &mut held
        ));

        let mut held = StopAllKey::default();
        assert!(!stop_all_press(key(ESCAPE, true, chord), None, &mut held));
        assert!(!held.is_down());
    }
}
