//! The assistant's turn in the Linux listener (`assistant::trigger`): what
//! the Action shortcut's press and release ask, Stop all tasks, and
//! Escape's stop. The capture itself is the listener's, as a dictation's:
//! the Action shortcut's release finishes it with the question's reply, so
//! its transcript goes to the assistant instead of being pasted or typed.

use crate::assistant::trigger::{Asking, Event, Step};
use crate::audio::CaptureInstant;
use crate::control::PrintReply;
use crate::dictation::DictationFlavour;
use crate::linux::input::HotkeyEvent;

/// Takes the assistant's part of `action` at `at` and returns what the
/// listener still does with it: a press becomes a start, a release a finish
/// whose transcript answers `print`. `owns_capture` says whether the Action
/// shortcut started the capture under way. `None` when nothing is left to
/// do: Stop all, Escape's stop, or a press before the assistant was set up,
/// which records nothing.
pub(super) fn take(
    asking: &mut Asking,
    action: HotkeyEvent,
    owns_capture: bool,
    at: CaptureInstant,
    print: &mut Option<PrintReply>,
) -> Option<HotkeyEvent> {
    let event = match action {
        HotkeyEvent::Ask { after_tap } => Event::Ask { after_tap },
        HotkeyEvent::StopAll => Event::StopAll,
        HotkeyEvent::StopHere => Event::StopHere,
        HotkeyEvent::DoubleTap => Event::DoubleTap,
        HotkeyEvent::Finish if owns_capture || asking.setting_up() => Event::Finish,
        HotkeyEvent::Cancel if owns_capture || asking.setting_up() => Event::Cancel,
        action => return Some(action),
    };
    match asking.take(event, at) {
        Step::Done => None,
        Step::Start => Some(HotkeyEvent::Start {
            flavour: DictationFlavour::Assistant,
        }),
        Step::Finish(reply) => {
            *print = Some(reply);
            Some(HotkeyEvent::Finish)
        }
        Step::Cancel => Some(HotkeyEvent::Cancel),
        Step::Discard => Some(HotkeyEvent::DoubleTap),
    }
}
