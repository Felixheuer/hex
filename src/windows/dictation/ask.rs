//! The assistant's turn in the Windows listener (`assistant::trigger`):
//! what the Action shortcut's press and release ask, Stop all tasks, and
//! Escape's stop. The capture itself is the listener's, as a dictation's:
//! the Action shortcut's release finishes it with the question's reply, so
//! its transcript goes to the assistant instead of being pasted.

use crate::assistant::trigger::{Asking, Event, Step};
use crate::audio::CaptureInstant;
use crate::control::PrintReply;
use crate::dictation::DictationFlavour;
use crate::windows::input::HotkeyAction;

/// Takes the assistant's part of `action` (a shortcut event of `flavour`
/// at `at`) and returns what the listener still does with it: a press
/// becomes a start, a release a finish whose transcript answers `print`.
/// `None` when nothing is left to do: Stop all, Escape's stop, or a press
/// before the assistant was set up, which records nothing.
pub(super) fn take(
    asking: &mut Asking,
    action: HotkeyAction,
    flavour: DictationFlavour,
    at: CaptureInstant,
    print: &mut Option<PrintReply>,
) -> Option<HotkeyAction> {
    let event = match action {
        HotkeyAction::Ask { after_tap } => Event::Ask { after_tap },
        HotkeyAction::StopAll => Event::StopAll,
        HotkeyAction::StopHere => Event::StopHere,
        HotkeyAction::DoubleTap => Event::DoubleTap,
        HotkeyAction::Finish if flavour.assistant() => Event::Finish,
        HotkeyAction::Cancel if flavour.assistant() => Event::Cancel,
        action => return Some(action),
    };
    match asking.take(event, at) {
        Step::Done => None,
        Step::Start => Some(HotkeyAction::Start),
        Step::Finish(reply) => {
            *print = Some(reply);
            Some(HotkeyAction::Finish)
        }
        Step::Cancel => Some(HotkeyAction::Cancel),
        Step::Discard => Some(HotkeyAction::DoubleTap),
    }
}
