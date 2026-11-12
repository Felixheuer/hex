//! How the assistant's moments overlap: what wins when the person presses,
//! speaks, or stops while Hex reads aloud, listens, or works. Every rule
//! below is carried out where its event arrives; this module holds the one
//! decision they share (which captures quiet Hex) and the table itself.
//!
//! | While …                     | … happens                          | then                                                        |
//! |-----------------------------|------------------------------------|-------------------------------------------------------------|
//! | Hex reads aloud             | a hold or a dictation becomes real | quiet at once, the rest of that answer dropped; inbox questions stay |
//! | Hex reads aloud             | the person talks over it           | it pauses; their words drop the answer and ask, nothing or Hex's own voice resumes it (barge-in) |
//! | anything                    | Escape or a spoken „Stopp“         | Here tasks and the reading stop                             |
//! | anything                    | Stop all                           | every task and the reading stop                             |
//! | conversation mode listening | a key or a hold                    | its capture closes first                                    |
//! | a capture is open           | another shortcut                   | ignored (one capture at a time)                             |
//! | any transcript              | it is Hex's own voice              | dropped (`speech::own_echo`)                                |
//! | a Here task thinks          | another hold                       | it steers that task                                         |
//! | a job runs                  | a hold names it                    | it goes to that job                                         |
//! | an inbox question waits     | a hold                             | it answers the question (the router decides)                |
//! | the person holds or dictates| jobs finish                        | their announcements wait their turn (`listening`)           |
//! | other audio plays           | an answer ends                     | no hands-free listening, no barge-in                        |
//! | any task works              | —                                  | the screen frame shows (`service::frame_shown`)             |

pub mod tuning;

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// A microphone capture the person starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capture {
    /// A hold of the Action shortcut.
    Hold,
    /// Hex's dictation shortcut.
    Dictation,
    /// Conversation mode's hands-free follow-up after an answer.
    FollowUp,
    /// A meeting recording.
    Meeting,
}

/// Whether `capture` quiets Hex the moment it becomes real: the person is
/// talking to the microphone, which would otherwise hear Hex reading. A
/// follow-up only opens once the reading ended, and a meeting records the
/// room, so neither cuts an answer.
pub fn hushes(capture: Capture) -> bool {
    matches!(capture, Capture::Hold | Capture::Dictation)
}

/// Hex goes quiet at once: the line being read and the rest of that answer;
/// questions and summaries waiting their turn stay.
pub fn hush() {
    crate::assistant::service::hush();
}

/// Whether Hex listens to the person now, and how: the one answer every
/// surface reads (the HUD's ring, the overlay, the voice queue that holds
/// its announcements back), so none says "listening" while another
/// doesn't. Barge-in's watch under an answer is not listening: it only
/// waits for a voice and shows nothing until one comes (`Hearing`).
///
/// Each reason is kept on its own ([`Reasons`]), so one capture ending never
/// clears another's: a hold, a dictation, and conversation mode's window
/// each set and clear only their own.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Listening {
    #[default]
    Off,
    /// The person holds the Action shortcut.
    Hold,
    /// The person dictates (Hex's dictation shortcut records).
    Dictation,
    /// Conversation mode's window after an answer is open, waiting.
    FollowUp,
    /// A follow-up or an interruption is being heard.
    Hearing,
}

impl Listening {
    fn bit(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Hold => HOLD,
            Self::Dictation => DICTATION,
            Self::FollowUp => FOLLOW_UP,
            Self::Hearing => HEARING,
        }
    }

    /// Whether spoken announcements wait (the person may be talking).
    pub fn holds_speech_back(self) -> bool {
        self != Self::Off
    }
}

const HOLD: u8 = 1;
const DICTATION: u8 = 2;
const FOLLOW_UP: u8 = 4;
const HEARING: u8 = 8;
/// Conversation mode's own reasons, replaced together.
const CONVERSATION: u8 = FOLLOW_UP | HEARING;

/// Every reason Hex listens now.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Reasons(u8);

impl Reasons {
    /// The one that shows: a hold over a dictation over hearing over the
    /// open window.
    pub fn shown(self) -> Listening {
        [
            Listening::Hold,
            Listening::Dictation,
            Listening::Hearing,
            Listening::FollowUp,
        ]
        .into_iter()
        .find(|reason| self.0 & reason.bit() != 0)
        .unwrap_or(Listening::Off)
    }

    fn with(self, reason: Listening, on: bool) -> Self {
        if on {
            Self(self.0 | reason.bit())
        } else {
            Self(self.0 & !reason.bit())
        }
    }

    /// Conversation mode moves to `to`: its own reasons are replaced, a
    /// hold's and a dictation's stay.
    fn conversation(self, to: Listening) -> Self {
        let kept = self.0 & !CONVERSATION;
        match to {
            Listening::FollowUp | Listening::Hearing => Self(kept | to.bit()),
            _ => Self(kept),
        }
    }
}

static REASONS: AtomicU8 = AtomicU8::new(0);
/// When a long capture last told the voice queue it still listens.
static REFRESHED_MS: AtomicU64 = AtomicU64::new(0);
/// How often a long capture renews the voice queue's hold-back, which
/// lapses on its own after 30 s should an end never arrive.
const REFRESH_EVERY_MS: u64 = 5_000;

/// What Hex listens to now.
pub fn listening() -> Listening {
    reasons().shown()
}

/// Every reason Hex listens now.
pub fn reasons() -> Reasons {
    Reasons(REASONS.load(Ordering::Acquire))
}

/// Conversation mode's capture moves to `to` (`FollowUp`, `Hearing`, or
/// `Off` when it closes or hands its words on). It never ends a hold or a
/// dictation.
pub fn conversation_listening(to: Listening) {
    update(|reasons| reasons.conversation(to));
}

/// A hold starts (`true`) or ends, however it ends (finished, discarded as
/// a chord or a tap, cancelled, or a double tap).
pub fn hold_listening(on: bool) {
    update(|reasons| reasons.with(Listening::Hold, on));
}

/// Whether a dictation records now. The listener calls it every time round
/// its loop, so a missed edge can't leave it on; a long dictation keeps
/// announcements waiting past the voice queue's own 30 s lapse.
pub fn dictation_listening(on: bool) {
    let before = reasons();
    if before.with(Listening::Dictation, on) != before {
        update(|reasons| reasons.with(Listening::Dictation, on));
    } else if on {
        let now = crate::assistant::store::now_ms();
        if now.saturating_sub(REFRESHED_MS.load(Ordering::Acquire)) >= REFRESH_EVERY_MS {
            REFRESHED_MS.store(now, Ordering::Release);
            crate::assistant::service::set_listening(true);
        }
    }
}

fn update(change: impl Fn(Reasons) -> Reasons) {
    let mut now = REASONS.load(Ordering::Acquire);
    loop {
        let next = change(Reasons(now)).0;
        match REASONS.compare_exchange_weak(now, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                REFRESHED_MS.store(crate::assistant::store::now_ms(), Ordering::Release);
                crate::assistant::service::set_listening(
                    Reasons(next).shown().holds_speech_back(),
                );
                return;
            }
            Err(actual) => now = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_and_a_dictation_quiet_hex_and_nothing_else_does() {
        assert!(hushes(Capture::Hold));
        assert!(hushes(Capture::Dictation));
        assert!(!hushes(Capture::FollowUp));
        assert!(!hushes(Capture::Meeting));
    }

    fn after(steps: &[(&str, Listening, bool)]) -> Reasons {
        steps
            .iter()
            .fold(Reasons::default(), |reasons, (who, what, on)| match *who {
                "conversation" => reasons.conversation(if *on { *what } else { Listening::Off }),
                _ => reasons.with(*what, *on),
            })
    }

    #[test]
    fn conversation_mode_never_ends_a_hold_or_a_dictation() {
        use Listening::*;
        let shown = |steps: &[(&str, Listening, bool)]| after(steps).shown();
        assert_eq!(shown(&[("conversation", FollowUp, true)]), FollowUp);
        assert_eq!(
            shown(&[("conversation", FollowUp, true), ("conversation", Hearing, true)]),
            Hearing
        );
        assert_eq!(
            shown(&[("conversation", Hearing, true), ("conversation", Off, false)]),
            Off
        );
        assert_eq!(shown(&[("hold", Hold, true), ("conversation", Off, false)]), Hold);
        assert_eq!(
            shown(&[("hold", Hold, true), ("conversation", FollowUp, true)]),
            Hold
        );
        assert_eq!(
            shown(&[("dictation", Dictation, true), ("conversation", Off, false)]),
            Dictation
        );
        assert!(!Off.holds_speech_back());
        assert!(
            FollowUp.holds_speech_back(),
            "no announcement into an open window"
        );
        assert!(
            Dictation.holds_speech_back(),
            "no announcement while the person dictates"
        );
    }

    #[test]
    fn one_capture_ending_never_clears_another() {
        use Listening::*;
        // The window opened, then a hold took over and ended: the window's
        // own close still has to come.
        let reasons = after(&[
            ("conversation", FollowUp, true),
            ("hold", Hold, true),
            ("hold", Hold, false),
        ]);
        assert_eq!(reasons.shown(), FollowUp);
        // A hold discarded as a ⌘C chord ends it like a finished one.
        let reasons = after(&[("hold", Hold, true), ("hold", Hold, false)]);
        assert_eq!(reasons.shown(), Off);
        // A dictation over an open window: both must end.
        let reasons = after(&[
            ("conversation", FollowUp, true),
            ("dictation", Dictation, true),
            ("conversation", Off, false),
        ]);
        assert_eq!(reasons.shown(), Dictation);
        assert_eq!(
            after(&[("dictation", Dictation, true), ("dictation", Dictation, false)]).shown(),
            Off
        );
    }
}
