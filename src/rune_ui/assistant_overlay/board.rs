//! What the assistant's overlays show, as plain state fed by
//! `assistant::Event`s: the tasks (for the stack and the window outlines)
//! and the quick question's bubble, with when each finished thing leaves.
//! No drawing and no platform here, so every rule is a unit test.

use std::time::{Duration, Instant};

use crate::assistant::{ConversationId, Event, Origin, TaskState, TaskStatus, TaskWindow};

use super::palette::COLOURS;

/// How long a finished task stays in the stack with its ✓ or ✗.
pub(crate) const FINISHED_LINGER: Duration = Duration::from_secs(5);
/// A stopped task leaves sooner: the person stopped it and knows.
pub(crate) const STOPPED_LINGER: Duration = Duration::from_millis(1500);
/// The longest a title, step, or outcome line runs before "…".
pub(crate) const LINE_CHARS: usize = 72;
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
/// A failed quick question's message stays as long as a failure notice.
const BUBBLE_FAILED: Duration = Duration::from_secs(6);

/// Where a task is in its life.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Queued,
    Running,
    /// Answered (✓).
    Done,
    /// Failed (✗) with its message.
    Failed,
    /// Cancelled by the person.
    Stopped,
}

impl Phase {
    pub(crate) fn live(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

/// One task as the stack and outlines draw it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Task {
    pub(crate) id: ConversationId,
    pub(crate) origin: Origin,
    /// The question, shortened to a line.
    pub(crate) title: String,
    pub(crate) colour: u8,
    pub(crate) phase: Phase,
    /// The latest step, as a person reads it.
    pub(crate) step: Option<String>,
    pub(crate) windows: Vec<TaskWindow>,
    /// The answer so far (streamed), or the final one.
    pub(crate) answer: String,
    /// Why it failed.
    pub(crate) error: Option<String>,
    /// When it finished, for how long it lingers.
    ended: Option<Instant>,
}

impl Task {
    /// The stack row's second line: the step while it runs, the answer's
    /// first line or the error once it's over.
    pub(crate) fn detail(&self) -> Option<String> {
        match self.phase {
            Phase::Queued | Phase::Running => self.step.clone(),
            Phase::Done => Some(first_line(&self.answer)).filter(|line| !line.is_empty()),
            Phase::Failed => self.error.as_deref().map(first_line),
            Phase::Stopped => None,
        }
    }

    fn leaves_at(&self) -> Option<Instant> {
        let linger = if self.phase == Phase::Stopped {
            STOPPED_LINGER
        } else {
            FINISHED_LINGER
        };
        self.ended.map(|ended| ended + linger)
    }
}

/// One window a live task works in, for its outline.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outlined {
    pub(crate) task: ConversationId,
    pub(crate) colour: u8,
    pub(crate) title: String,
    pub(crate) window: TaskWindow,
}

/// Every task the overlays know of, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Board {
    pub(crate) tasks: Vec<Task>,
}

impl Board {
    /// The tasks running or queued when the overlays started listening.
    pub(crate) fn seeded(statuses: Vec<TaskStatus>) -> Self {
        let tasks = statuses
            .into_iter()
            .map(|status| Task {
                id: status.conversation,
                origin: status.origin,
                title: shorten(&status.title),
                colour: status.colour % COLOURS,
                phase: match status.state {
                    TaskState::Queued => Phase::Queued,
                    TaskState::Running => Phase::Running,
                },
                step: status.step.as_deref().map(shorten),
                windows: status.windows,
                answer: String::new(),
                error: None,
                ended: None,
            })
            .collect();
        Self { tasks }
    }

    /// Applies `event`; `colour` is the service's colour for a task it
    /// starts (`TaskStatus.colour`), when it has one. Whether anything the
    /// overlays draw changed.
    pub(crate) fn apply(&mut self, event: &Event, colour: Option<u8>, now: Instant) -> bool {
        match event {
            Event::Queued {
                conversation,
                origin,
                question,
            }
            | Event::Started {
                conversation,
                origin,
                question,
            } => {
                let phase = if matches!(event, Event::Queued { .. }) {
                    Phase::Queued
                } else {
                    Phase::Running
                };
                if let Some(task) = self.task_mut(conversation) {
                    // A follow-up in the same conversation starts it again.
                    task.phase = phase;
                    task.ended = None;
                    task.error = None;
                    task.answer.clear();
                    if let Some(colour) = colour {
                        task.colour = colour % COLOURS;
                    }
                    return true;
                }
                let colour = colour.map_or_else(|| self.free_colour(), |c| c % COLOURS);
                self.tasks.push(Task {
                    id: conversation.clone(),
                    origin: *origin,
                    title: shorten(question),
                    colour,
                    phase,
                    step: None,
                    windows: Vec::new(),
                    answer: String::new(),
                    error: None,
                    ended: None,
                });
                true
            }
            Event::Thinking { conversation } => self.live(conversation, |task| {
                task.phase = Phase::Running;
                true
            }),
            Event::Step {
                conversation,
                description,
            } => self.live(conversation, |task| {
                task.phase = Phase::Running;
                task.step = Some(shorten(description));
                true
            }),
            Event::Window {
                conversation,
                window,
            } => self.live(conversation, |task| {
                if task.windows.contains(window) {
                    return false;
                }
                task.windows.push(window.clone());
                true
            }),
            Event::AnswerDelta { conversation, text } => self.live(conversation, |task| {
                task.answer.push_str(text);
                // Only the stack's finished line reads the answer.
                false
            }),
            Event::Answer {
                conversation, text, ..
            } => self.finish(conversation, Phase::Done, now, |task| {
                task.answer = text.clone();
            }),
            Event::Failed {
                conversation,
                message,
            } => self.finish(conversation, Phase::Failed, now, |task| {
                task.error = Some(message.clone());
            }),
            Event::Cancelled { conversation } => {
                self.finish(conversation, Phase::Stopped, now, |_| {})
            }
            Event::ConversationsChanged | Event::ModeChanged { .. } => false,
        }
    }

    /// Drops finished tasks whose time is up; whether any went.
    pub(crate) fn expire(&mut self, now: Instant) -> bool {
        let before = self.tasks.len();
        self.tasks
            .retain(|task| task.leaves_at().is_none_or(|at| now < at));
        self.tasks.len() != before
    }

    /// When the next finished task leaves.
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.tasks.iter().filter_map(Task::leaves_at).min()
    }

    /// The Agent mode tasks the stack lists, oldest first.
    pub(crate) fn stack(&self) -> Vec<Task> {
        self.tasks
            .iter()
            .filter(|task| task.origin == Origin::AgentMode)
            .cloned()
            .collect()
    }

    /// Every live task's windows, whatever started it, for their outlines.
    pub(crate) fn outlined(&self) -> Vec<Outlined> {
        self.tasks
            .iter()
            .filter(|task| task.phase.live())
            .flat_map(|task| {
                task.windows.iter().map(|window| Outlined {
                    task: task.id.clone(),
                    colour: task.colour,
                    title: task.title.clone(),
                    window: window.clone(),
                })
            })
            .collect()
    }

    fn task_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    /// Runs `change` on a live task named `id`; events for a task that's
    /// over or unknown change nothing.
    fn live(&mut self, id: &str, change: impl FnOnce(&mut Task) -> bool) -> bool {
        match self.task_mut(id) {
            Some(task) if task.phase.live() => change(task),
            _ => false,
        }
    }

    fn finish(
        &mut self,
        id: &str,
        phase: Phase,
        now: Instant,
        change: impl FnOnce(&mut Task),
    ) -> bool {
        let Some(task) = self.task_mut(id) else {
            return false;
        };
        task.phase = phase;
        task.ended = Some(now);
        task.step = None;
        change(task);
        true
    }

    /// The lowest colour no live task wears, for a task the service gave
    /// none (so a fixture or a late subscriber still tells tasks apart).
    fn free_colour(&self) -> u8 {
        (0..COLOURS)
            .find(|colour| {
                !self
                    .tasks
                    .iter()
                    .any(|task| task.phase.live() && task.colour == *colour)
            })
            .unwrap_or(self.tasks.len() as u8 % COLOURS)
    }
}

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
    pub(crate) phase: BubblePhase,
    /// The latest [`BUBBLE_STEPS`] steps, oldest first.
    pub(crate) steps: Vec<String>,
    pub(crate) answer: String,
    pub(crate) error: Option<String>,
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
            phase: BubblePhase::Thinking,
            steps: Vec::new(),
            answer: String::new(),
            error: None,
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
                if bubble
                    .as_ref()
                    .is_some_and(|b| &b.conversation == conversation && b.ended.is_none())
                {
                    // Started after Queued: the same bubble.
                    return false;
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

/// `text` as one line of at most [`LINE_CHARS`] characters: its first
/// non-empty line, whitespace collapsed, cut at a word, with "…" when
/// anything was left out.
pub(crate) fn shorten(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = lines.next().unwrap_or_default();
    clip(line, lines.next().is_some(), LINE_CHARS)
}

/// The first non-empty line of an answer or error, shortened.
pub(crate) fn first_line(text: &str) -> String {
    shorten(text)
}

fn clip(line: &str, more: bool, max: usize) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let joined = words.join(" ");
    if joined.chars().count() <= max && !more {
        return joined;
    }
    let mut out = String::new();
    for word in &words {
        let next = if out.is_empty() {
            word.chars().count()
        } else {
            out.chars().count() + 1 + word.chars().count()
        };
        if next > max - 1 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        // One word longer than the line.
        out = joined.chars().take(max - 1).collect();
    }
    let out = out.trim_end_matches([',', ';', ':', '.', ' ']);
    format!("{out}…")
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

    fn window(app: &str) -> TaskWindow {
        TaskWindow {
            app: app.into(),
            window_id: None,
        }
    }

    #[test]
    fn a_task_runs_steps_and_lingers_after_its_answer() {
        let t0 = Instant::now();
        let mut board = Board::default();
        assert!(board.apply(
            &Event::Queued {
                conversation: id(1),
                origin: Origin::AgentMode,
                question: "Book a table for two tonight".into(),
            },
            Some(3),
            t0,
        ));
        assert_eq!(board.tasks[0].phase, Phase::Queued);
        assert_eq!(board.tasks[0].colour, 3);
        board.apply(&started(1, Origin::AgentMode, "Book a table"), None, t0);
        assert_eq!(
            board.tasks.len(),
            1,
            "Started after Queued is the same task"
        );
        assert_eq!(board.tasks[0].phase, Phase::Running);
        assert_eq!(board.tasks[0].colour, 3, "keeps its colour");
        board.apply(
            &Event::Step {
                conversation: id(1),
                description: "Opened Safari".into(),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].detail().as_deref(), Some("Opened Safari"));
        board.apply(
            &Event::Window {
                conversation: id(1),
                window: window("Safari"),
            },
            None,
            t0,
        );
        assert!(
            !board.apply(
                &Event::Window {
                    conversation: id(1),
                    window: window("Safari"),
                },
                None,
                t0,
            ),
            "the same window once"
        );
        assert_eq!(board.outlined().len(), 1);
        board.apply(
            &Event::Answer {
                conversation: id(1),
                origin: Origin::AgentMode,
                text: "\nBooked Nopa for 8 pm.\nConfirmation sent to your email.".into(),
            },
            None,
            t0 + Duration::from_secs(10),
        );
        let task = &board.tasks[0];
        assert_eq!(task.phase, Phase::Done);
        assert_eq!(task.detail().as_deref(), Some("Booked Nopa for 8 pm…"));
        assert!(
            board.outlined().is_empty(),
            "a finished task's outline goes"
        );
        let leaves = board.next_expiry().unwrap();
        assert_eq!(leaves, t0 + Duration::from_secs(10) + FINISHED_LINGER);
        assert!(!board.expire(leaves - Duration::from_millis(1)));
        assert!(board.expire(leaves));
        assert!(board.tasks.is_empty());
    }

    #[test]
    fn a_stopped_task_leaves_sooner_and_late_events_change_nothing() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::AgentMode, "Tidy my desktop"), None, t0);
        board.apply(
            &Event::Cancelled {
                conversation: id(1),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].phase, Phase::Stopped);
        assert!(!board.apply(
            &Event::Step {
                conversation: id(1),
                description: "Moved a file".into(),
            },
            None,
            t0,
        ));
        assert_eq!(board.next_expiry(), Some(t0 + STOPPED_LINGER));
        assert!(!board.apply(
            &Event::Thinking {
                conversation: id(9)
            },
            None,
            t0
        ));
    }

    #[test]
    fn the_stack_lists_agent_mode_tasks_and_outlines_cover_every_live_one() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::AgentMode, "a"), None, t0);
        board.apply(&started(2, Origin::Chat, "b"), None, t0);
        board.apply(&started(3, Origin::ActionShortcut, "c"), None, t0);
        for n in 1..=3 {
            board.apply(
                &Event::Window {
                    conversation: id(n),
                    window: window("Notes"),
                },
                None,
                t0,
            );
        }
        let stack: Vec<_> = board.stack().into_iter().map(|t| t.id).collect();
        assert_eq!(stack, [id(1)]);
        assert_eq!(board.outlined().len(), 3);
    }

    #[test]
    fn tasks_without_a_colour_take_the_first_free_one() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::AgentMode, "a"), Some(0), t0);
        board.apply(&started(2, Origin::AgentMode, "b"), None, t0);
        board.apply(&started(3, Origin::AgentMode, "c"), None, t0);
        let colours: Vec<_> = board.tasks.iter().map(|t| t.colour).collect();
        assert_eq!(colours, [0, 1, 2]);
        board.apply(
            &Event::Cancelled {
                conversation: id(2),
            },
            None,
            t0,
        );
        board.apply(&started(4, Origin::AgentMode, "d"), None, t0);
        assert_eq!(
            board.tasks[3].colour, 1,
            "a finished task's colour is free again"
        );
        board.apply(&started(5, Origin::AgentMode, "e"), Some(11), t0);
        assert_eq!(board.tasks[4].colour, 11 % COLOURS);
    }

    #[test]
    fn titles_and_lines_fit_one_line() {
        assert_eq!(shorten("  Open   Mail  "), "Open Mail");
        assert_eq!(shorten("First line\nsecond"), "First line…");
        let long = "Find the cheapest flight from Berlin to Lisbon next weekend and put it in my calendar please";
        let short = shorten(long);
        assert!(short.chars().count() <= LINE_CHARS, "{short}");
        assert!(short.ends_with('…'));
        assert!(long.starts_with(short.trim_end_matches('…')));
        assert_eq!(first_line("\n\n  Done.  \nMore"), "Done…");
        assert_eq!(first_line("All set"), "All set");
        let word = "x".repeat(200);
        assert_eq!(shorten(&word).chars().count(), LINE_CHARS);
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
            &started(1, Origin::AgentMode, "x"),
            t0
        ));
        assert!(bubble.is_none(), "Agent mode tasks never show the bubble");
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
    fn a_failed_quick_question_shows_its_error_for_six_seconds() {
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
        assert_eq!(b.due(t0, true), Due::At(t0 + BUBBLE_FAILED));
        assert_eq!(b.due(t0 + BUBBLE_FAILED, false), Due::Leave);
    }

    #[test]
    fn reading_time_grows_with_the_answer_up_to_a_cap() {
        assert_eq!(reading_time(""), Duration::ZERO);
        assert_eq!(reading_time("one two three four"), Duration::from_secs(1));
        assert_eq!(reading_time(&"word ".repeat(1000)), READING_MAX);
    }
}
