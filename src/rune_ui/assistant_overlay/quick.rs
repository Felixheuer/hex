//! The Here question's bubble as plain state fed by `assistant::Event`s:
//! its thinking, steps, streamed answer, question, or escalation, and when
//! it leaves (`Bubble::due`). No drawing here, so every rule is a unit test.

use std::time::{Duration, Instant};

use crate::assistant::{ConversationId, Event, Origin};

use super::board::shorten;
use super::failure::Failure;

/// How many of a quick question's latest steps its bubble lists.
pub(crate) const BUBBLE_STEPS: usize = 3;
/// A quick answer stays at least this long once it's done (or once its
/// speech ends).
const BUBBLE_LINGER: Duration = Duration::from_secs(4);
/// Reading time added per word of a quick answer that isn't read aloud
/// (about 240 words a minute) …
const READING_PER_WORD: Duration = Duration::from_millis(250);
/// … up to this much.
const READING_MAX: Duration = Duration::from_secs(20);
/// A failed quick question's line stays long enough to hear it and press
/// its Fix or Try again.
const BUBBLE_FAILED: Duration = Duration::from_secs(12);

/// What the quick question's bubble shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BubblePhase {
    /// Waiting on the model, nothing done yet.
    Thinking,
    /// Doing things on the desktop; its steps list.
    Working,
    /// The answer streams in.
    Answering,
    Answered,
    Failed,
}

/// The quick question's bubble beside the HUD.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Bubble {
    pub(crate) conversation: ConversationId,
    pub(crate) question: String,
    /// The question in full, for Try again.
    pub(crate) asked: String,
    /// The language it was asked in (`assistant::lines::language`): what
    /// Hex shows and says back about it is in that one.
    pub(crate) language: Option<&'static str>,
    pub(crate) phase: BubblePhase,
    /// The latest [`BUBBLE_STEPS`] steps, oldest first.
    pub(crate) steps: Vec<String>,
    pub(crate) answer: String,
    pub(crate) error: Option<String>,
    /// Why it failed, read from `error`: its line and its one action.
    pub(crate) failure: Option<Failure>,
    /// What the task asks the person, waiting for the answer.
    pub(crate) asks: Option<String>,
    /// It became a Background task (`Event::Escalated`): it flies to the
    /// orb and goes.
    pub(crate) escalated: bool,
    ended: Option<Instant>,
    /// Its answer was read aloud at some point.
    spoke: bool,
    /// When reading aloud was last seen to have stopped, after the end.
    quiet_since: Option<Instant>,
}

/// What a bubble wants next.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Due {
    /// Stay; nothing timed yet (the question is still being answered).
    Stay,
    /// Look again at this time.
    At(Instant),
    /// Leave now.
    Leave,
}

impl Bubble {
    pub(crate) fn new(conversation: ConversationId, question: &str) -> Self {
        Self {
            conversation,
            question: shorten(question),
            asked: question.trim().to_owned(),
            language: crate::assistant::lines::language(question),
            phase: BubblePhase::Thinking,
            steps: Vec::new(),
            answer: String::new(),
            error: None,
            failure: None,
            asks: None,
            escalated: false,
            ended: None,
            spoke: false,
            quiet_since: None,
        }
    }

    /// The quick question's bubble after `event`: a new quick question
    /// replaces the old one's, a cancelled one goes, and others' events
    /// pass by. Whether it changed.
    pub(crate) fn apply(bubble: &mut Option<Self>, event: &Event, now: Instant) -> bool {
        match event {
            Event::Queued {
                conversation,
                origin: Origin::ActionShortcut,
                question,
            }
            | Event::Started {
                conversation,
                origin: Origin::ActionShortcut,
                question,
            } => {
                if let Some(b) = bubble
                    .as_mut()
                    .filter(|b| &b.conversation == conversation && b.ended.is_none())
                {
                    // Started after Queued, or after answering its
                    // question: the same bubble.
                    return b.asks.take().is_some();
                }
                *bubble = Some(Self::new(conversation.clone(), question));
                true
            }
            Event::Cancelled { conversation } => {
                if bubble
                    .as_ref()
                    .is_some_and(|b| &b.conversation == conversation)
                {
                    *bubble = None;
                    return true;
                }
                false
            }
            other => {
                let Some(b) = bubble
                    .as_mut()
                    .filter(|b| Some(&b.conversation) == other.conversation())
                else {
                    return false;
                };
                b.step(other, now)
            }
        }
    }

    fn step(&mut self, event: &Event, now: Instant) -> bool {
        if self.ended.is_some() {
            return false;
        }
        match event {
            Event::Thinking { .. } => false,
            Event::Step { description, .. } => {
                if self.phase == BubblePhase::Thinking {
                    self.phase = BubblePhase::Working;
                }
                self.steps.push(shorten(description));
                let extra = self.steps.len().saturating_sub(BUBBLE_STEPS);
                self.steps.drain(..extra);
                true
            }
            Event::AnswerDelta { text, .. } => {
                self.phase = BubblePhase::Answering;
                self.answer.push_str(text);
                true
            }
            Event::Answer { text, .. } => {
                self.phase = BubblePhase::Answered;
                self.answer = text.trim().to_owned();
                self.ended = Some(now);
                true
            }
            Event::Failed { message, .. } => {
                self.phase = BubblePhase::Failed;
                self.error = Some(message.clone());
                self.failure = Some(Failure::of(message));
                self.ended = Some(now);
                true
            }
            Event::Question { text, .. } => {
                self.asks = Some(text.trim().to_owned());
                true
            }
            Event::Escalated { .. } => {
                self.escalated = true;
                if !self.answer.trim().is_empty() {
                    self.phase = BubblePhase::Answered;
                }
                self.ended = Some(now);
                true
            }
            _ => false,
        }
    }

    /// Whether the question is over (answered or failed).
    pub(crate) fn over(&self) -> bool {
        self.ended.is_some()
    }

    /// When the bubble leaves: never while the question is answered, nor
    /// while its answer is read aloud; then [`BUBBLE_LINGER`] after the
    /// speech, or, unread, the linger plus reading time from the answer.
    pub(crate) fn due(&mut self, now: Instant, speaking: bool) -> Due {
        let Some(ended) = self.ended else {
            return Due::Stay;
        };
        // Handed to the background with nothing to show: it flies to the
        // orb at once. With its spoken line ("On it: …") it stays until
        // that was read, then goes.
        if self.escalated && self.answer.trim().is_empty() {
            return Due::Leave;
        }
        if self.phase == BubblePhase::Failed {
            return at_or_leave(ended + BUBBLE_FAILED, now);
        }
        if speaking {
            self.spoke = true;
            self.quiet_since = None;
            // Speech has no end event to wait on: look again soon.
            return Due::At(now + Duration::from_millis(400));
        }
        let leaves = if self.spoke {
            *self.quiet_since.get_or_insert(now) + BUBBLE_LINGER
        } else {
            ended + BUBBLE_LINGER + reading_time(&self.answer)
        };
        at_or_leave(leaves, now)
    }
}

fn at_or_leave(at: Instant, now: Instant) -> Due {
    if now >= at { Due::Leave } else { Due::At(at) }
}

/// How long `text` takes to read, capped.
pub(crate) fn reading_time(text: &str) -> Duration {
    let words = text.split_whitespace().count() as u32;
    (READING_PER_WORD * words).min(READING_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> ConversationId {
        format!("c{n}")
    }

    fn started(n: u32, origin: Origin, question: &str) -> Event {
        Event::Started {
            conversation: id(n),
            origin,
            question: question.into(),
        }
    }

    fn quick(n: u32) -> Event {
        started(n, Origin::ActionShortcut, "What's the weather like?")
    }

    #[test]
    fn a_quick_question_thinks_steps_streams_and_answers() {
        let t0 = Instant::now();
        let mut bubble = None;
        assert!(!Bubble::apply(
            &mut bubble,
            &started(1, Origin::Background, "x"),
            t0
        ));
        assert!(bubble.is_none(), "Background tasks never show the bubble");
        assert!(Bubble::apply(&mut bubble, &quick(1), t0));
        assert_eq!(bubble.as_ref().unwrap().phase, BubblePhase::Thinking);
        for n in 0..5 {
            Bubble::apply(
                &mut bubble,
                &Event::Step {
                    conversation: id(1),
                    description: format!("Step {n}"),
                },
                t0,
            );
        }
        let b = bubble.as_ref().unwrap();
        assert_eq!(b.phase, BubblePhase::Working);
        assert_eq!(b.steps, ["Step 2", "Step 3", "Step 4"]);
        for part in ["Sunny, ", "22 °C."] {
            Bubble::apply(
                &mut bubble,
                &Event::AnswerDelta {
                    conversation: id(1),
                    text: part.into(),
                },
                t0,
            );
        }
        assert_eq!(bubble.as_ref().unwrap().answer, "Sunny, 22 °C.");
        assert_eq!(bubble.as_mut().unwrap().due(t0, false), Due::Stay);
        Bubble::apply(
            &mut bubble,
            &Event::Answer {
                conversation: id(1),
                origin: Origin::ActionShortcut,
                text: "Sunny, 22 °C. ".into(),
            },
            t0,
        );
        let b = bubble.as_mut().unwrap();
        assert_eq!(b.phase, BubblePhase::Answered);
        assert_eq!(b.answer, "Sunny, 22 °C.");
        let unread = BUBBLE_LINGER + reading_time("Sunny, 22 °C.");
        assert_eq!(b.due(t0, false), Due::At(t0 + unread));
        assert_eq!(b.due(t0 + unread, false), Due::Leave);
    }

    #[test]
    fn a_spoken_answer_stays_until_four_seconds_after_the_speech() {
        let t0 = Instant::now();
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(1), t0);
        Bubble::apply(
            &mut bubble,
            &Event::Answer {
                conversation: id(1),
                origin: Origin::ActionShortcut,
                text: "A long answer read aloud.".into(),
            },
            t0,
        );
        let b = bubble.as_mut().unwrap();
        let later = t0 + Duration::from_secs(30);
        assert!(
            matches!(b.due(later, true), Due::At(_)),
            "stays while speaking"
        );
        let quiet = later + Duration::from_secs(1);
        assert_eq!(b.due(quiet, false), Due::At(quiet + BUBBLE_LINGER));
        assert_eq!(
            b.due(quiet + Duration::from_secs(1), false),
            Due::At(quiet + BUBBLE_LINGER),
            "measured from when the speech stopped"
        );
        assert_eq!(b.due(quiet + BUBBLE_LINGER, false), Due::Leave);
    }

    #[test]
    fn a_new_quick_question_replaces_the_bubble_and_cancel_removes_it() {
        let t0 = Instant::now();
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(1), t0);
        assert!(
            !Bubble::apply(
                &mut bubble,
                &Event::Queued {
                    conversation: id(1),
                    origin: Origin::ActionShortcut,
                    question: "again".into(),
                },
                t0,
            ),
            "Queued then Started is one bubble"
        );
        Bubble::apply(&mut bubble, &quick(2), t0);
        assert_eq!(bubble.as_ref().unwrap().conversation, id(2));
        assert!(
            !Bubble::apply(
                &mut bubble,
                &Event::Step {
                    conversation: id(1),
                    description: "old".into(),
                },
                t0,
            ),
            "the replaced question's events pass by"
        );
        assert!(!Bubble::apply(
            &mut bubble,
            &Event::Cancelled {
                conversation: id(1)
            },
            t0
        ));
        assert!(Bubble::apply(
            &mut bubble,
            &Event::Cancelled {
                conversation: id(2)
            },
            t0
        ));
        assert!(bubble.is_none());
    }

    #[test]
    fn a_failed_quick_question_shows_why_until_it_can_be_acted_on() {
        let t0 = Instant::now();
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(1), t0);
        Bubble::apply(
            &mut bubble,
            &Event::Failed {
                conversation: id(1),
                message: "Claude Code isn't signed in.".into(),
            },
            t0,
        );
        let b = bubble.as_mut().unwrap();
        assert_eq!(b.phase, BubblePhase::Failed);
        assert_eq!(b.error.as_deref(), Some("Claude Code isn't signed in."));
        assert_eq!(b.failure, Some(Failure::SignIn));
        assert_eq!(b.due(t0, true), Due::At(t0 + BUBBLE_FAILED));
        assert_eq!(b.due(t0 + BUBBLE_FAILED, false), Due::Leave);
    }

    #[test]
    fn reading_time_grows_with_the_answer_up_to_a_cap() {
        assert_eq!(reading_time(""), Duration::ZERO);
        assert_eq!(reading_time("one two three four"), Duration::from_secs(1));
        assert_eq!(reading_time(&"word ".repeat(1000)), READING_MAX);
    }

    /// Every line Hex says about a Here question shows: the answer, and the
    /// "On it: ..." said when it hands the question to the background, which
    /// stays while it's read and only then flies to the orb.
    #[test]
    fn a_spoken_hand_off_line_stays_visible_until_it_was_read() {
        let t0 = Instant::now();
        let mut bubble = None;
        assert!(Bubble::apply(&mut bubble, &quick(1), t0));
        let line = Event::AnswerDelta {
            conversation: id(1),
            text: "Mach ich: drei Aufträge laufen.".into(),
        };
        assert!(Bubble::apply(&mut bubble, &line, t0));
        assert!(Bubble::apply(
            &mut bubble,
            &Event::Escalated {
                conversation: id(1)
            },
            t0
        ));
        let b = bubble.as_mut().unwrap();
        assert!(b.escalated);
        assert_eq!(b.answer, "Mach ich: drei Aufträge laufen.");
        // Read aloud: it stays.
        assert!(matches!(b.due(t0, true), Due::At(_)));
        // Read, then quiet past the linger: it goes (flying to the orb).
        let later = t0 + Duration::from_secs(1);
        assert!(matches!(b.due(later, false), Due::At(_)));
        assert_eq!(b.due(later + BUBBLE_LINGER, false), Due::Leave);
    }

    /// With nothing to say, a handed-off question flies to the orb at once.
    #[test]
    fn a_silent_hand_off_flies_at_once() {
        let t0 = Instant::now();
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(2), t0);
        Bubble::apply(
            &mut bubble,
            &Event::Escalated {
                conversation: id(2),
            },
            t0,
        );
        assert_eq!(bubble.as_mut().unwrap().due(t0, true), Due::Leave);
    }

    /// The spoken status reply ("Wie weit ist ...?") is a Here question with
    /// its answer: it shows as one.
    #[test]
    fn a_spoken_status_reply_shows_as_a_bubble() {
        let t0 = Instant::now();
        let mut bubble = None;
        assert!(Bubble::apply(&mut bubble, &quick(3), t0));
        assert!(Bubble::apply(
            &mut bubble,
            &Event::Answer {
                conversation: id(3),
                origin: Origin::ActionShortcut,
                text: "Der Shop ist zu drei Vierteln fertig.".into(),
            },
            t0
        ));
        let b = bubble.as_ref().unwrap();
        assert_eq!(b.phase, BubblePhase::Answered);
        assert!(!b.answer.is_empty());
    }
}
