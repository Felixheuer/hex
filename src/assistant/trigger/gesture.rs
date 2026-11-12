//! Tells the Action shortcut's gestures apart from its presses and
//! releases, for listeners whose input reports just those (the Windows
//! keyboard hook, X11 grabs, the Wayland shortcuts portal). The rules are
//! macOS's (`suppression::DictationHotkey` with a double-tap gesture):
//!
//! - every press starts a capture; it follows a tap when the last press
//!   was a tap (released before the hold threshold) that came up within the
//!   double-tap window;
//! - a press that follows a tap and comes up before the hold threshold is
//!   the double tap: its capture ends unheard and nothing is left armed;
//! - any other release finishes its capture, and only a plain tap arms the
//!   next press. A hold, or a tap then a hold, never pairs with the press
//!   after it.

use std::time::{Duration, Instant};

use crate::dictation::MINIMUM_HOLD_DURATION;

/// How soon after a tap the next press counts as its second, as for the
/// dictation shortcut's double tap.
pub(crate) const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(300);

/// What a press or release of the Action shortcut does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GestureAction {
    /// Start a capture; `after_tap` when a tap came just before (held, it
    /// asks a new Background task).
    Start { after_tap: bool },
    /// The capture ends: ask with what was said (a short tap's capture is
    /// too short to transcribe and asks nothing).
    Finish,
    /// The second of two taps came up: its capture ends unheard, and the
    /// task list shows or hides.
    DoubleTap,
}

/// The Action shortcut's state between its presses and releases.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Gesture {
    #[default]
    Idle,
    /// Up; the last press was a tap that came up at this time.
    Tapped(Instant),
    /// Down since this time.
    Down { since: Instant, after_tap: bool },
}

impl Gesture {
    /// The shortcut went down at `now`. A press while it is down (a key's
    /// repeat) does nothing.
    pub(crate) fn press(&mut self, now: Instant) -> Option<GestureAction> {
        let after_tap = match *self {
            Self::Down { .. } => return None,
            Self::Idle => false,
            Self::Tapped(released) => now
                .checked_duration_since(released)
                .is_some_and(|gap| gap < DOUBLE_TAP_WINDOW),
        };
        *self = Self::Down {
            since: now,
            after_tap,
        };
        Some(GestureAction::Start { after_tap })
    }

    /// The shortcut came up at `now`. A release without its press does
    /// nothing.
    pub(crate) fn release(&mut self, now: Instant) -> Option<GestureAction> {
        let Self::Down { since, after_tap } = *self else {
            return None;
        };
        let tap = now.saturating_duration_since(since) < MINIMUM_HOLD_DURATION;
        if after_tap && tap {
            *self = Self::Idle;
            return Some(GestureAction::DoubleTap);
        }
        *self = if tap && !after_tap {
            Self::Tapped(now)
        } else {
            Self::Idle
        };
        Some(GestureAction::Finish)
    }

    /// The press was taken away (a chord, the listener lost track of the
    /// keys, a cancel): nothing stays armed. Returns whether it was down,
    /// so its capture needs cancelling.
    pub(crate) fn cancel(&mut self) -> bool {
        let down = self.is_down();
        *self = Self::Idle;
        down
    }

    /// Whether the shortcut is down now.
    pub(crate) fn is_down(&self) -> bool {
        matches!(self, Self::Down { .. })
    }
}
