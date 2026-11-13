//! The Here question's bubble as plain state fed by `assistant::Event`s:
//! its thinking, what it understood, its plan and steps as reasoning lines
//! ([`Bubble::reasoning`]), streamed answer, question, or escalation, and
//! when it leaves (`Bubble::due`); and the words heard while the Action
//! shortcut is still held ([`Held`]). No drawing here, so every rule is a
//! unit test.

use std::time::{Duration, Instant};

use crate::assistant::{ConversationId, Event, Origin, PlanStep};

use super::failure::Failure;
use crate::assistant::view::shorten;

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
    /// Agent-Desk shows and says back about it is in that one.
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
    /// What the agent understood the question as (`Event::Understood`).
    pub(crate) understood: Option<String>,
    /// Its plan, when it made one (`Event::Plan`).
    pub(crate) plan: Vec<PlanStep>,
    /// When its first step came: the reasoning folds to Now and Next
    /// [`REASONING_FOLDS`] after.
    working_since: Option<Instant>,
    /// The reasoning shows only Now and Next.
    pub(crate) folded: bool,
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
            understood: None,
            plan: Vec::new(),
            working_since: None,
            folded: false,
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
            Event::Understood { text, .. } => {
                let text = text.trim();
                if text.is_empty() || self.understood.as_deref() == Some(text) {
                    return false;
                }
                self.understood = Some(text.to_owned());
                true
            }
            Event::Plan { steps, .. } => {
                if &self.plan == steps {
                    return false;
                }
                self.plan = steps.clone();
                true
            }
            Event::Step { description, .. } => {
                if self.phase == BubblePhase::Thinking {
                    self.phase = BubblePhase::Working;
                }
                self.working_since.get_or_insert(now);
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

    /// Folds the reasoning to Now and Next once the turn has worked for
    /// [`REASONING_FOLDS`]: whether it folded now, and when to look again.
    pub(crate) fn fold(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let Some(since) = self.working_since.filter(|_| !self.folded) else {
            return (false, None);
        };
        if self.phase != BubblePhase::Working {
            return (false, None);
        }
        let at = since + REASONING_FOLDS;
        if now >= at {
            self.folded = true;
            (true, None)
        } else {
            (false, Some(at))
        }
    }

    /// What the card shows of the agent's reasoning while it thinks and
    /// works, top-down, at most [`REASONING_LINES`]: **Understood** (what
    /// it understood the question as), **Plan** (its plan's steps in one
    /// line), **Now** (the step it is on) and **Next** (the one after);
    /// without a plan, the steps it took (ticked) before the one it is on.
    /// Folded ([`Bubble::fold`]), only Now and Next. Nothing once its
    /// answer streams, it asks, or it is over.
    pub(crate) fn reasoning(&self) -> Vec<Reason> {
        if !matches!(self.phase, BubblePhase::Thinking | BubblePhase::Working)
            || self.asks.is_some()
        {
            return Vec::new();
        }
        let started = self.working_since.is_some() || self.plan.iter().any(|step| step.done);
        let mut lines = Vec::new();
        if let Some(understood) = &self.understood {
            lines.push(Reason::new(Tag::Understood, understood, Mark::Done));
        }
        if !self.plan.is_empty() {
            let titles: Vec<&str> = self.plan.iter().map(|step| step.title.trim()).collect();
            let plan = shorten_to(&titles.join(" → "), REASON_CHARS);
            lines.push(Reason::new(
                Tag::Plan,
                &plan,
                if started { Mark::Done } else { Mark::Now },
            ));
            let mut open = self.plan.iter().filter(|step| !step.done);
            if started && let Some(now) = open.next() {
                lines.push(Reason::new(Tag::Now, &now.title, Mark::Now));
            }
            if let Some(next) = open.next() {
                lines.push(Reason::new(Tag::Next, &next.title, Mark::Next));
            }
        } else if let Some((now, earlier)) = self.steps.split_last() {
            for step in earlier {
                lines.push(Reason::new(Tag::Earlier, step, Mark::Done));
            }
            lines.push(Reason::new(Tag::Now, now, Mark::Now));
        }
        if self.folded {
            lines.retain(|line| matches!(line.tag, Tag::Now | Tag::Next));
        }
        // The latest lines when there are more: Now stays, the oldest go.
        let extra = lines.len().saturating_sub(REASONING_LINES);
        lines.drain(..extra);
        lines
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

/// The most reasoning lines a card shows (`docs/ux/redesign.md` §2).
pub(crate) const REASONING_LINES: usize = 4;
/// A reasoning line's most characters.
pub(crate) const REASON_CHARS: usize = 80;
/// How long the reasoning shows in full once the turn works; then only
/// Now and Next.
pub(crate) const REASONING_FOLDS: Duration = Duration::from_secs(4);

/// Which reasoning line it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tag {
    Understood,
    Plan,
    /// A step taken before the one it is on (a turn without a plan).
    Earlier,
    Now,
    Next,
}

/// Where a reasoning line stands: passed (✓), now, or still to come.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mark {
    Done,
    Now,
    Next,
}

/// One reasoning line: which, its words (at most [`REASON_CHARS`]), and
/// where it stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Reason {
    pub(crate) tag: Tag,
    pub(crate) words: String,
    pub(crate) mark: Mark,
}

impl Reason {
    fn new(tag: Tag, words: &str, mark: Mark) -> Self {
        Self {
            tag,
            words: shorten_to(words.trim(), REASON_CHARS),
            mark,
        }
    }
}

/// `text` cut to `max` characters with "…".
fn shorten_to(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// What the person is saying while they still hold the Action shortcut:
/// the app in front (which a quick turn is about) and the words heard so
/// far, settled and tentative, as the dictation's live text gives them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Held {
    pub(crate) app: Option<String>,
    /// Words that won't change any more.
    pub(crate) words: String,
    /// The last words, still being recognised.
    pub(crate) tentative: String,
}

impl Held {
    pub(crate) fn new(app: Option<String>) -> Self {
        Self {
            app,
            ..Self::default()
        }
    }

    /// The live text now: whether the card's words changed.
    pub(crate) fn hear(&mut self, stable: &str, tentative: &str) -> bool {
        let (stable, tentative) = (stable.trim(), tentative.trim());
        if self.words == stable && self.tentative == tentative {
            return false;
        }
        self.words = stable.to_owned();
        self.tentative = tentative.to_owned();
        true
    }

    /// Nothing heard yet.
    pub(crate) fn silent(&self) -> bool {
        self.words.is_empty() && self.tentative.is_empty()
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

    fn understood(n: u32, text: &str) -> Event {
        Event::Understood {
            conversation: id(n),
            text: text.into(),
        }
    }

    fn plan(n: u32, done: usize, titles: &[&str]) -> Event {
        Event::Plan {
            conversation: id(n),
            steps: titles
                .iter()
                .enumerate()
                .map(|(index, title)| PlanStep {
                    title: (*title).into(),
                    done: index < done,
                })
                .collect(),
        }
    }

    fn tags(bubble: &Bubble) -> Vec<(Tag, Mark)> {
        bubble
            .reasoning()
            .iter()
            .map(|line| (line.tag, line.mark))
            .collect()
    }

    #[test]
    fn the_understood_line_is_shown_before_the_first_step() {
        let t0 = Instant::now();
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(1), t0);
        assert!(bubble.as_ref().unwrap().reasoning().is_empty());
        assert!(Bubble::apply(
            &mut bubble,
            &understood(1, "Your calendar, this afternoon"),
            t0
        ));
        let b = bubble.as_ref().unwrap();
        assert_eq!(b.phase, BubblePhase::Thinking, "before it acts");
        assert_eq!(b.reasoning()[0].tag, Tag::Understood);
        assert_eq!(b.reasoning()[0].words, "Your calendar, this afternoon");
        assert!(!Bubble::apply(
            &mut bubble,
            &understood(1, "Your calendar, this afternoon"),
            t0
        ));
        Bubble::apply(
            &mut bubble,
            &Event::Step {
                conversation: id(1),
                description: "Opened Calendar".into(),
            },
            t0,
        );
        assert_eq!(
            tags(bubble.as_ref().unwrap()),
            [(Tag::Understood, Mark::Done), (Tag::Now, Mark::Now)],
            "understood stays first, ticked, above the step"
        );
    }

    #[test]
    fn reasoning_shows_at_most_four_lines() {
        let t0 = Instant::now();
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(1), t0);
        Bubble::apply(&mut bubble, &understood(1, "Nopa, 2 people, 8 pm"), t0);
        let titles = ["Open Resy", "Search Nopa", "Pick 8 pm", "Ask before paying"];
        Bubble::apply(&mut bubble, &plan(1, 0, &titles), t0);
        assert_eq!(
            tags(bubble.as_ref().unwrap()),
            [
                (Tag::Understood, Mark::Done),
                (Tag::Plan, Mark::Now),
                (Tag::Next, Mark::Next)
            ],
            "planned, nothing started"
        );
        Bubble::apply(&mut bubble, &plan(1, 1, &titles), t0);
        let b = bubble.as_ref().unwrap();
        assert_eq!(
            tags(b),
            [
                (Tag::Understood, Mark::Done),
                (Tag::Plan, Mark::Done),
                (Tag::Now, Mark::Now),
                (Tag::Next, Mark::Next)
            ]
        );
        assert_eq!(b.reasoning()[2].words, "Search Nopa");
        assert_eq!(b.reasoning()[3].words, "Pick 8 pm");
        // Without a plan, many steps: still four, the latest.
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(2), t0);
        Bubble::apply(&mut bubble, &understood(2, "x"), t0);
        for n in 0..6 {
            Bubble::apply(
                &mut bubble,
                &Event::Step {
                    conversation: id(2),
                    description: format!("Step {n}"),
                },
                t0,
            );
        }
        let lines = bubble.as_ref().unwrap().reasoning();
        assert!(lines.len() <= REASONING_LINES, "{lines:?}");
        assert_eq!(lines.last().unwrap().tag, Tag::Now);
        assert_eq!(lines.last().unwrap().words, "Step 5");
        // Long words are cut.
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(3), t0);
        Bubble::apply(&mut bubble, &understood(3, &"word ".repeat(40)), t0);
        let line = &bubble.as_ref().unwrap().reasoning()[0];
        assert!(line.words.chars().count() <= REASON_CHARS);
    }

    #[test]
    fn the_reasoning_folds_to_now_and_next_after_four_seconds_of_work() {
        let t0 = Instant::now();
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(1), t0);
        Bubble::apply(&mut bubble, &understood(1, "Nopa, 2 people, 8 pm"), t0);
        Bubble::apply(
            &mut bubble,
            &plan(1, 1, &["Open Resy", "Search", "Pick"]),
            t0,
        );
        let b = bubble.as_mut().unwrap();
        assert_eq!(b.fold(t0), (false, None), "thinking: nothing folds");
        Bubble::apply(
            &mut bubble,
            &Event::Step {
                conversation: id(1),
                description: "Opened Resy".into(),
            },
            t0,
        );
        let b = bubble.as_mut().unwrap();
        assert_eq!(b.fold(t0), (false, Some(t0 + REASONING_FOLDS)));
        assert_eq!(b.reasoning().len(), 4);
        assert_eq!(b.fold(t0 + REASONING_FOLDS), (true, None));
        assert_eq!(tags(b), [(Tag::Now, Mark::Now), (Tag::Next, Mark::Next)]);
        assert_eq!(b.fold(t0 + REASONING_FOLDS * 2), (false, None), "once");
    }

    #[test]
    fn live_words_update_while_held() {
        let mut held = Held::new(Some("Mail".into()));
        assert!(held.silent());
        assert!(held.hear("Book Nopa", "for"));
        assert_eq!(
            (held.words.as_str(), held.tentative.as_str()),
            ("Book Nopa", "for")
        );
        assert!(
            !held.hear("Book Nopa ", " for"),
            "the same words redraw nothing"
        );
        assert!(held.hear("Book Nopa for two at eight", ""));
        assert_eq!(held.words, "Book Nopa for two at eight");
        assert!(!held.silent());
        assert_eq!(held.app.as_deref(), Some("Mail"));
    }

    #[test]
    fn reading_time_grows_with_the_answer_up_to_a_cap() {
        assert_eq!(reading_time(""), Duration::ZERO);
        assert_eq!(reading_time("one two three four"), Duration::from_secs(1));
        assert_eq!(reading_time(&"word ".repeat(1000)), READING_MAX);
    }

    /// Every line Agent-Desk says about a Here question shows: the answer, and the
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
