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
//! | the person holds or dictates| jobs finish                        | their announcements wait their turn                         |
//! | other audio plays           | an answer ends                     | no hands-free listening, no barge-in                        |
//! | any task works              | —                                  | the screen frame shows (`service::frame_shown`)             |

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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Listening {
    #[default]
    Off,
    /// The person holds the Action shortcut.
    Hold,
    /// Conversation mode's window after an answer is open, waiting.
    FollowUp,
    /// A follow-up or an interruption is being heard.
    Hearing,
}

impl Listening {
    fn code(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Hold => 1,
            Self::FollowUp => 2,
            Self::Hearing => 3,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Hold,
            2 => Self::FollowUp,
            3 => Self::Hearing,
            _ => Self::Off,
        }
    }

    /// Whether spoken announcements wait (the person may be talking).
    pub fn holds_speech_back(self) -> bool {
        self != Self::Off
    }
}

static LISTENING: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// What Hex listens to now.
pub fn listening() -> Listening {
    Listening::from_code(LISTENING.load(std::sync::atomic::Ordering::Acquire))
}

/// Conversation mode's capture moves to `to` (`FollowUp`, `Hearing`, or
/// `Off` when it closes or hands its words on). It never ends a hold.
pub fn conversation_listening(to: Listening) {
    let next = after_conversation(listening(), to);
    set(next);
}

/// A hold starts (`true`) or ends; a hold takes over whatever conversation
/// mode had open.
pub fn hold_listening(on: bool) {
    set(if on { Listening::Hold } else { Listening::Off });
}

fn set(to: Listening) {
    LISTENING.store(to.code(), std::sync::atomic::Ordering::Release);
    crate::assistant::service::set_listening(to.holds_speech_back());
}

/// The state after conversation mode asks for `to` while `now` holds: a
/// hold in progress stays (its own end clears it).
fn after_conversation(now: Listening, to: Listening) -> Listening {
    if now == Listening::Hold { now } else { to }
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

    #[test]
    fn conversation_mode_never_ends_a_hold() {
        use Listening::*;
        assert_eq!(after_conversation(Off, FollowUp), FollowUp);
        assert_eq!(after_conversation(FollowUp, Hearing), Hearing);
        assert_eq!(after_conversation(Hearing, Off), Off);
        assert_eq!(after_conversation(Hold, Off), Hold);
        assert_eq!(after_conversation(Hold, FollowUp), Hold);
        assert!(!Off.holds_speech_back());
        assert!(
            FollowUp.holds_speech_back(),
            "no announcement into an open window"
        );
    }
}
