//! How the assistant's moments overlap: what wins when the person presses,
//! speaks, or stops while Hex reads aloud, listens, or works. Every rule
//! below is carried out where its event arrives; this module holds the one
//! decision they share (which captures quiet Hex) and the table itself.
//!
//! | While …                     | … happens                          | then                                                        |
//! |-----------------------------|------------------------------------|-------------------------------------------------------------|
//! | Hex reads aloud             | a hold or a dictation becomes real | quiet at once, the rest of that answer dropped; inbox questions stay |
//! | Hex reads aloud             | the person talks over it           | it stops; their words are the next request (barge-in)       |
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
}
