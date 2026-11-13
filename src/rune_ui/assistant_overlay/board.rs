//! What the assistant's overlays show, as plain state fed by
//! `assistant::Event`s: the tasks (for the orb, its list, and the window
//! outlines), the bubbles that pop from the orb ([`Pops`]), and the Here
//! question's bubble, with when each finished thing leaves. No drawing and
//! no platform here, so every rule is a unit test.

use std::time::{Duration, Instant};

use crate::assistant::{
    ConversationId, Event, Origin, PlanStep, TaskState, TaskStatus, TaskWindow, WaitingQuestion,
};

use super::palette::COLOURS;
use super::pops::Pops;
pub(crate) use super::quick::{Bubble, BubblePhase, Due};

/// How long a finished task stays in the stack with its ✓ or ✗.
pub(crate) const FINISHED_LINGER: Duration = Duration::from_secs(5);
/// A stopped task leaves sooner: the person stopped it and knows.
pub(crate) const STOPPED_LINGER: Duration = Duration::from_millis(1500);
/// The longest a title, step, or outcome line runs before "…".
pub(crate) const LINE_CHARS: usize = 72;

/// Where a task is in its life.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Queued,
    Running,
    /// The person took it over ("You have it"); its desktop actions wait.
    Paused,
    /// Hit its agent's usage limit; retries at this Unix ms time.
    Waiting {
        until_ms: u64,
    },
    /// Asked the person something and waits for the answer.
    NeedsInput,
    /// Answered (✓).
    Done,
    /// Failed (✗) with its message.
    Failed,
    /// Cancelled by the person.
    Stopped,
}

impl Phase {
    pub(crate) fn live(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Paused | Self::Waiting { .. } | Self::NeedsInput
        )
    }

    fn of(state: TaskState) -> Self {
        match state {
            TaskState::Queued => Self::Queued,
            TaskState::Running => Self::Running,
            TaskState::Paused => Self::Paused,
            TaskState::Waiting { until_ms } => Self::Waiting { until_ms },
            TaskState::NeedsInput => Self::NeedsInput,
        }
    }
}

/// Where a task works, for the small glyph on its row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Place {
    /// On the web: a hidden browser session, or a browser's windows.
    Web,
    /// In apps' windows on the desktop.
    Desktop,
    /// Nothing yet says.
    Unknown,
}

/// Browsers by the names agent-desktop gives their apps.
const BROWSERS: [&str; 9] = [
    "safari",
    "google chrome",
    "chrome",
    "chromium",
    "arc",
    "firefox",
    "brave browser",
    "microsoft edge",
    "agent-browser",
];

/// Words in a step that mean the web.
const WEB_WORDS: [&str; 8] = [
    "http://",
    "https://",
    "www.",
    "website",
    "web page",
    "browser",
    "searched the web",
    "url",
];

/// How many of a task's latest steps the expanded list shows.
pub(crate) const TASK_STEPS: usize = 3;

/// One task as the overlays draw it.
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
    /// The latest [`TASK_STEPS`] steps, oldest first.
    pub(crate) steps: Vec<String>,
    pub(crate) windows: Vec<TaskWindow>,
    /// The answer so far (streamed), or the final one.
    pub(crate) answer: String,
    /// Why it failed.
    pub(crate) error: Option<String>,
    /// It moves the real pointer or presses real keys now ("Hands off").
    pub(crate) hands_off: bool,
    /// What it asked the person and waits on, until answered or dismissed.
    pub(crate) question: Option<String>,
    /// When it asked, in the board's order of questions (newest highest).
    pub(crate) asked: u64,
    /// Its plan, once it made one.
    pub(crate) plan: Vec<PlanStep>,
    /// When it finished, for how long it lingers.
    ended: Option<Instant>,
    /// When it started (or a follow-up started it again), as far as the
    /// overlays saw it: a job's card says how long it runs.
    pub(crate) since: Option<Instant>,
    /// The job it helps (`assistant::agent::team`): its card goes under
    /// that job's.
    pub(crate) lead: Option<ConversationId>,
    /// When it last worked on the screen (`Event::Acting`): the screen
    /// frame shows for [`ACTING_FOR`] after.
    pub(crate) acting_at: Option<Instant>,
}

/// How long the screen frame stays after a task last acted on the screen.
pub(crate) const ACTING_FOR: Duration = Duration::from_secs(4);

impl Task {
    fn new(id: ConversationId, origin: Origin, title: &str, colour: u8, phase: Phase) -> Self {
        Self {
            id,
            origin,
            title: shorten(title),
            colour,
            phase,
            step: None,
            steps: Vec::new(),
            windows: Vec::new(),
            answer: String::new(),
            error: None,
            hands_off: false,
            question: None,
            asked: 0,
            plan: Vec::new(),
            ended: None,
            since: None,
            lead: None,
            acting_at: None,
        }
    }

    /// A row's second line: the step while it runs, the answer's first line
    /// or the error once it's over.
    pub(crate) fn detail(&self) -> Option<String> {
        match self.phase {
            Phase::Queued
            | Phase::Running
            | Phase::Paused
            | Phase::Waiting { .. }
            | Phase::NeedsInput => self.step.clone(),
            Phase::Done => Some(first_line(&self.answer)).filter(|line| !line.is_empty()),
            Phase::Failed => self.error.as_deref().map(first_line),
            Phase::Stopped => None,
        }
    }

    /// Where it works: the web when its windows are all browsers, or it has
    /// none and a step names the web; the desktop when it has windows.
    pub(crate) fn place(&self) -> Place {
        let browser = |w: &TaskWindow| BROWSERS.contains(&w.app.to_lowercase().as_str());
        if !self.windows.is_empty() {
            return if self.windows.iter().all(browser) {
                Place::Web
            } else {
                Place::Desktop
            };
        }
        let web = self.steps.iter().chain(&self.step).any(|step| {
            let step = step.to_lowercase();
            WEB_WORDS.iter().any(|word| step.contains(word))
        });
        if web { Place::Web } else { Place::Unknown }
    }

    /// Its plan's steps done, and in all, once it has a plan.
    pub(crate) fn progress(&self) -> Option<(usize, usize)> {
        (!self.plan.is_empty()).then(|| {
            let done = self.plan.iter().filter(|step| step.done).count();
            (done, self.plan.len())
        })
    }

    /// The plan step it's on: the first not done.
    pub(crate) fn current_step(&self) -> Option<usize> {
        self.plan.iter().position(|step| !step.done)
    }

    fn note_step(&mut self, description: &str) {
        let step = shorten(description);
        if self.steps.last() != Some(&step) {
            self.steps.push(step.clone());
            let extra = self.steps.len().saturating_sub(TASK_STEPS);
            self.steps.drain(..extra);
        }
        self.step = Some(step);
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

/// How a window's outline looks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Look {
    /// Its task works there: a solid line in its colour.
    #[default]
    Working,
    /// The person has it: a dashed line and "You have it".
    Paused,
    /// Waiting out a usage limit: dimmed, with when it resumes.
    Waiting { until_ms: u64 },
    /// A task moves the real pointer there: a soft line.
    HandsOff,
}

/// One window a live task works in, for its outline.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outlined {
    pub(crate) task: ConversationId,
    pub(crate) colour: u8,
    pub(crate) title: String,
    pub(crate) window: TaskWindow,
    pub(crate) look: Look,
    /// Its plan's steps done and in all, for the title chip.
    pub(crate) progress: Option<(usize, usize)>,
}

/// The question the orb's bubble shows: the inbox's active one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Asking {
    pub(crate) id: ConversationId,
    /// Its task's title, for "… asks".
    pub(crate) title: String,
    pub(crate) question: String,
    pub(crate) colour: u8,
}

/// The service's inbox as last told (`Event::QuestionsChanged`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InboxView {
    pub(crate) active: Option<WaitingQuestion>,
    /// Questions behind the active one.
    pub(crate) waiting: usize,
}

/// Every task the overlays know of, oldest first, and the bubbles that pop
/// from the orb about them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Board {
    pub(crate) tasks: Vec<Task>,
    pub(crate) pops: Pops,
    /// Questions asked so far, to order them newest first.
    asked: u64,
    /// The inbox, once the service told it; before that the newest question
    /// stands for the active one.
    pub(crate) inbox: Option<InboxView>,
    /// The active question whose bubble the person put away (×); it comes
    /// back when another becomes active.
    hidden: Option<ConversationId>,
}

impl Board {
    /// The tasks running or queued when the overlays started listening.
    pub(crate) fn seeded(statuses: Vec<TaskStatus>) -> Self {
        let tasks = statuses
            .into_iter()
            .map(|status| {
                let mut task = Task::new(
                    status.conversation,
                    status.origin,
                    &status.title,
                    status.colour % COLOURS,
                    Phase::of(status.state),
                );
                if let Some(step) = &status.step {
                    task.note_step(step);
                }
                task.windows = status.windows;
                task
            })
            .collect();
        Self {
            tasks,
            pops: Pops::default(),
            asked: 0,
            inbox: None,
            hidden: None,
        }
    }

    /// Applies `event`; `colour` is the service's colour for a task it
    /// starts (`TaskStatus.colour`), when it has one. Whether anything the
    /// overlays draw changed.
    pub(crate) fn apply(&mut self, event: &Event, colour: Option<u8>, now: Instant) -> bool {
        let fresh = event
            .conversation()
            .is_none_or(|id| self.task(id).is_none_or(|task| !task.phase.live()));
        let changed = self.apply_task(event, colour, now);
        let popped = match event.conversation().and_then(|id| self.task(id)) {
            Some(task) if task.origin == Origin::Background => {
                let task = task.clone();
                self.pops.apply(event, &task, fresh, now)
            }
            _ => false,
        };
        changed || popped
    }

    fn apply_task(&mut self, event: &Event, colour: Option<u8>, now: Instant) -> bool {
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
                    if task.since.is_none() || task.ended.is_some() {
                        task.since = Some(now);
                    }
                    // Answered: a follow-up is how an answer arrives.
                    task.question = None;
                    if let Some(colour) = colour {
                        task.colour = colour % COLOURS;
                    }
                    return true;
                }
                let colour = colour.map_or_else(|| self.free_colour(), |c| c % COLOURS);
                let mut task = Task::new(conversation.clone(), *origin, question, colour, phase);
                task.since = Some(now);
                task.lead = crate::assistant::agent::team::lead_of(conversation);
                self.tasks.push(task);
                true
            }
            Event::Thinking { conversation } => self.live(conversation, |task| {
                let was = task.phase;
                task.phase = resumed(task.phase);
                task.phase != was
            }),
            Event::Step {
                conversation,
                description,
            } => self.live(conversation, |task| {
                task.phase = resumed(task.phase);
                task.note_step(description);
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
                // Only the finished line reads the answer.
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
            Event::Paused { conversation } => {
                self.live(conversation, |task| set(&mut task.phase, Phase::Paused))
            }
            Event::Resumed { conversation } => {
                self.live(conversation, |task| set(&mut task.phase, Phase::Running))
            }
            Event::Waiting {
                conversation,
                until_ms,
                ..
            } => self.live(conversation, |task| {
                set(
                    &mut task.phase,
                    Phase::Waiting {
                        until_ms: *until_ms,
                    },
                )
            }),
            Event::HandsOff { conversation, on } => {
                self.live(conversation, |task| set(&mut task.hands_off, *on))
            }
            // Working on the screen: a desktop action by any task, or a
            // question about the screen reading a window.
            Event::Acting {
                conversation,
                looking,
            } => self.live(conversation, |task| {
                if *looking && task.origin.lane() != crate::assistant::Lane::Here {
                    return false;
                }
                let was_on = task.acting_at.is_some_and(|at| now < at + ACTING_FOR);
                task.acting_at = Some(now);
                !was_on
            }),
            Event::Plan {
                conversation,
                steps,
            } => self.live(conversation, |task| set(&mut task.plan, steps.clone())),
            Event::Question { conversation, text } => {
                self.asked += 1;
                let asked = self.asked;
                self.live(conversation, |task| {
                    task.phase = Phase::NeedsInput;
                    task.question = Some(text.trim().to_owned());
                    task.asked = asked;
                    true
                })
            }
            Event::Escalated { conversation } => self.live(conversation, |task| {
                // The Here question continues as a Background task.
                set(&mut task.origin, Origin::Background)
            }),
            Event::QuestionsChanged { active, waiting } => {
                let view = InboxView {
                    active: active.clone(),
                    waiting: *waiting,
                };
                let id = |view: Option<&InboxView>| {
                    view.and_then(|view| view.active.as_ref())
                        .map(|active| active.conversation.clone())
                };
                if id(self.inbox.as_ref()) != id(Some(&view)) {
                    self.hidden = None;
                }
                set(&mut self.inbox, Some(view))
            }
            Event::ConversationsChanged | Event::ModeChanged { .. } => false,
        }
    }

    /// Drops finished tasks and bubbles whose time is up; whether any went.
    pub(crate) fn expire(&mut self, now: Instant) -> bool {
        let before = self.tasks.len();
        self.tasks
            .retain(|task| task.leaves_at().is_none_or(|at| now < at));
        let popped = self.pops.expire(now);
        self.tasks.len() != before || popped
    }

    /// When the next finished task or bubble leaves.
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.tasks
            .iter()
            .filter_map(Task::leaves_at)
            .chain(self.pops.next_expiry())
            .min()
    }

    /// The Background tasks the orb and its list show, oldest first.
    pub(crate) fn stack(&self) -> Vec<Task> {
        self.tasks
            .iter()
            // Jobs: a Chats request that became one was escalated, which
            // made it Background here; a plain Chats reply isn't one.
            .filter(|task| task.origin == Origin::Background)
            .cloned()
            .collect()
    }

    /// Whether the orb has anything to show: a Background task, or a
    /// bubble about one.
    pub(crate) fn orb_shows(&self) -> bool {
        !self.pops.items.is_empty()
            || self
                .tasks
                .iter()
                .any(|task| task.origin == Origin::Background)
    }

    /// Whether any task waits out a usage limit (its countdown ticks).
    pub(crate) fn waiting(&self) -> bool {
        self.tasks
            .iter()
            .any(|task| matches!(task.phase, Phase::Waiting { .. }))
    }

    /// The live task moving the real pointer, if one is.
    pub(crate) fn hands_off(&self) -> Option<&Task> {
        self.tasks
            .iter()
            .find(|task| task.hands_off && task.phase.live())
    }

    /// Until when the screen frame shows: while a live task works on the
    /// screen (a desktop action within [`ACTING_FOR`], or real input under
    /// "Hands off"). Background work in hidden browsers or files shows no
    /// frame; its card at the orb is enough. `None`: no frame.
    pub(crate) fn frame_until(&self, now: Instant) -> Option<Instant> {
        self.tasks
            .iter()
            .filter(|task| task.phase.live())
            .filter_map(|task| {
                if task.hands_off {
                    return Some(now + ACTING_FOR);
                }
                task.acting_at
                    .map(|at| at + ACTING_FOR)
                    .filter(|until| *until > now)
            })
            .max()
    }

    /// Every live task's windows, whatever started it, for their outlines.
    pub(crate) fn outlined(&self) -> Vec<Outlined> {
        self.tasks
            .iter()
            .filter(|task| task.phase.live())
            .flat_map(|task| {
                let look = match task.phase {
                    Phase::Paused => Look::Paused,
                    Phase::Waiting { until_ms } => Look::Waiting { until_ms },
                    _ if task.hands_off => Look::HandsOff,
                    _ => Look::Working,
                };
                task.windows.iter().map(move |window| Outlined {
                    task: task.id.clone(),
                    colour: task.colour,
                    title: task.title.clone(),
                    window: window.clone(),
                    look,
                    progress: task.progress(),
                })
            })
            .collect()
    }

    /// The live tasks waiting on a question for the person, newest first.
    pub(crate) fn questions(&self) -> Vec<&Task> {
        let mut asking: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|task| task.phase.live() && task.question.is_some())
            .collect();
        asking.sort_by_key(|task| std::cmp::Reverse(task.asked));
        asking
    }

    /// Puts away `id`'s question bubble (it still waits for the answer);
    /// whether one was up.
    pub(crate) fn dismiss_question(&mut self, id: &str) -> bool {
        if self.inbox.is_some() {
            let active = self.active_question().is_some_and(|asking| asking.id == id);
            if active {
                self.hidden = Some(id.to_owned());
            }
            return active;
        }
        self.task_mut(id)
            .is_some_and(|task| task.question.take().is_some())
    }

    /// The question the orb's bubble shows: the inbox's active one (a
    /// Background task's), unless the person put it away; before the
    /// service told the inbox, the newest question.
    pub(crate) fn active_question(&self) -> Option<Asking> {
        let asking = match &self.inbox {
            Some(inbox) => {
                let active = inbox.active.as_ref()?;
                let task = self.task(&active.conversation);
                if task.is_some_and(|task| task.origin != Origin::Background) {
                    return None;
                }
                Asking {
                    id: active.conversation.clone(),
                    title: task.map_or_else(|| shorten(&active.title), |task| task.title.clone()),
                    question: active.question.trim().to_owned(),
                    colour: task.map_or(0, |task| task.colour),
                }
            }
            None => {
                let task = self
                    .questions()
                    .into_iter()
                    .find(|task| task.origin == Origin::Background)?;
                Asking {
                    id: task.id.clone(),
                    title: task.title.clone(),
                    question: task.question.clone().unwrap_or_default(),
                    colour: task.colour,
                }
            }
        };
        (self.hidden.as_deref() != Some(asking.id.as_str())).then_some(asking)
    }

    /// Every question waiting on the person, the active one included: the
    /// orb's badge.
    pub(crate) fn open_questions(&self) -> usize {
        match &self.inbox {
            Some(inbox) => inbox.waiting + usize::from(inbox.active.is_some()),
            None => self.questions().len(),
        }
    }

    pub(crate) fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == id)
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
        task.hands_off = false;
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

/// Sets `slot` to `value`; whether it changed.
fn set<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

/// A waiting task that thinks or steps again is running; a paused one stays
/// paused until it's handed back.
fn resumed(phase: Phase) -> Phase {
    match phase {
        Phase::Queued | Phase::Waiting { .. } => Phase::Running,
        // Still waiting on the person; their answer starts it again.
        Phase::NeedsInput => Phase::NeedsInput,
        other => other,
    }
}

/// `text` as one line of at most [`LINE_CHARS`] characters: its first
/// non-empty line, whitespace collapsed, cut at a word, with "…" when
/// anything was left out.
pub(crate) fn shorten(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = lines.next().unwrap_or_default();
    clip(line, lines.next().is_some(), LINE_CHARS)
}

/// The first non-empty line of an answer or error as plain text (its
/// Markdown marks gone), shortened.
pub(crate) fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(plain)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let more = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .nth(1)
        .is_some();
    clip(&line, more, LINE_CHARS)
}

/// One Markdown line as a person reads it: no heading, list, or quote
/// marker, no bold, italic, or code marks, and a link's text without its
/// address.
pub(crate) fn plain(line: &str) -> String {
    let mut line = line.trim();
    line = line.trim_start_matches('#').trim_start();
    line = line.trim_start_matches('>').trim_start();
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            line = rest;
        }
    }
    if let Some((number, rest)) = line.split_once(". ")
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
    {
        line = rest;
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    // Links: `[text](address)` reads as `text`.
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|at| open + at) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|at| close + at) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .replace(['*'], "")
        .trim()
        .to_owned()
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
                origin: Origin::Background,
                question: "Book a table for two tonight".into(),
            },
            Some(3),
            t0,
        ));
        assert_eq!(board.tasks[0].phase, Phase::Queued);
        assert_eq!(board.tasks[0].colour, 3);
        board.apply(&started(1, Origin::Background, "Book a table"), None, t0);
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
                origin: Origin::Background,
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
        let leaves = t0 + Duration::from_secs(10) + FINISHED_LINGER;
        let tick = board.next_expiry().unwrap();
        assert!(tick < leaves, "its ✓ bubble dissolves first");
        assert!(board.expire(tick));
        assert_eq!(board.tasks.len(), 1, "the list still shows it");
        assert_eq!(board.next_expiry(), Some(leaves));
        assert!(!board.expire(leaves - Duration::from_millis(1)));
        assert!(board.expire(leaves));
        assert!(board.tasks.is_empty());
    }

    #[test]
    fn a_stopped_task_leaves_sooner_and_late_events_change_nothing() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "Tidy my desktop"), None, t0);
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
    fn the_stack_lists_jobs_and_outlines_cover_every_live_one() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "a"), None, t0);
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
        // Jobs only: a plain Chats reply (`Chat`) and a quick question
        // aren't; a Chats request that became a job was escalated to
        // Background.
        assert_eq!(stack, [id(1)]);
        assert_eq!(board.outlined().len(), 3);
    }

    #[test]
    fn tasks_without_a_colour_take_the_first_free_one() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "a"), Some(0), t0);
        board.apply(&started(2, Origin::Background, "b"), None, t0);
        board.apply(&started(3, Origin::Background, "c"), None, t0);
        let colours: Vec<_> = board.tasks.iter().map(|t| t.colour).collect();
        assert_eq!(colours, [0, 1, 2]);
        board.apply(
            &Event::Cancelled {
                conversation: id(2),
            },
            None,
            t0,
        );
        board.apply(&started(4, Origin::Background, "d"), None, t0);
        assert_eq!(
            board.tasks[3].colour, 1,
            "a finished task's colour is free again"
        );
        board.apply(&started(5, Origin::Background, "e"), Some(11), t0);
        assert_eq!(board.tasks[4].colour, 11 % COLOURS);
    }

    #[test]
    fn a_finished_line_reads_without_its_markdown() {
        assert_eq!(first_line("## **Booked** a table"), "Booked a table");
        assert_eq!(
            first_line("- Run `cargo build`\n- then test"),
            "Run cargo build…"
        );
        assert_eq!(
            first_line("See [the menu](https://nopa.com) *tonight*"),
            "See the menu tonight"
        );
        assert_eq!(first_line("1. First step"), "First step");
        assert_eq!(plain("> quoted"), "quoted");
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
    fn a_task_taken_over_waits_for_its_hand_back_and_its_outline_says_so() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "Book a table"), None, t0);
        board.apply(
            &Event::Window {
                conversation: id(1),
                window: window("Safari"),
            },
            None,
            t0,
        );
        assert!(board.apply(
            &Event::Paused {
                conversation: id(1)
            },
            None,
            t0
        ));
        assert_eq!(board.tasks[0].phase, Phase::Paused);
        assert_eq!(board.outlined()[0].look, Look::Paused);
        board.apply(
            &Event::Step {
                conversation: id(1),
                description: "Looked again".into(),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].phase, Phase::Paused, "only Resumed ends it");
        assert!(board.apply(
            &Event::Resumed {
                conversation: id(1)
            },
            None,
            t0
        ));
        assert_eq!(board.tasks[0].phase, Phase::Running);
        assert_eq!(board.outlined()[0].look, Look::Working);
        assert!(!board.apply(
            &Event::Resumed {
                conversation: id(1)
            },
            None,
            t0
        ));
    }

    #[test]
    fn a_task_waiting_out_a_limit_dims_until_it_runs_again() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "Summarise"), None, t0);
        board.apply(
            &Event::Window {
                conversation: id(1),
                window: window("Notes"),
            },
            None,
            t0,
        );
        board.apply(
            &Event::Waiting {
                conversation: id(1),
                until_ms: 1_700_000_000_000,
                reason: "Claude's usage limit".into(),
            },
            None,
            t0,
        );
        assert!(board.waiting());
        assert_eq!(
            board.outlined()[0].look,
            Look::Waiting {
                until_ms: 1_700_000_000_000
            }
        );
        assert!(board.tasks[0].phase.live(), "still stoppable");
        board.apply(
            &Event::Thinking {
                conversation: id(1),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].phase, Phase::Running);
        assert!(!board.waiting());
    }

    #[test]
    fn hands_off_follows_its_task_and_ends_with_it() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&quick(1), None, t0);
        assert!(board.apply(
            &Event::HandsOff {
                conversation: id(1),
                on: true
            },
            None,
            t0
        ));
        assert_eq!(board.hands_off().map(|t| t.id.clone()), Some(id(1)));
        board.apply(
            &Event::Answer {
                conversation: id(1),
                origin: Origin::ActionShortcut,
                text: "Done".into(),
            },
            None,
            t0,
        );
        assert!(board.hands_off().is_none(), "a finished task lets go");
    }

    #[test]
    fn questions_wait_for_an_answer_newest_first_and_plans_count_their_steps() {
        let t0 = Instant::now();
        let mut board = Board::default();
        for n in 1..=2 {
            board.apply(&started(n, Origin::Background, "a"), None, t0);
            board.apply(
                &Event::Question {
                    conversation: id(n),
                    text: format!("Question {n}? "),
                },
                None,
                t0,
            );
        }
        assert_eq!(board.tasks[0].phase, Phase::NeedsInput);
        let asking: Vec<_> = board.questions().iter().map(|t| t.id.clone()).collect();
        assert_eq!(asking, [id(2), id(1)], "newest first");
        assert_eq!(board.tasks[0].question.as_deref(), Some("Question 1?"));
        board.apply(
            &Event::Step {
                conversation: id(1),
                description: "Still looking".into(),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].phase, Phase::NeedsInput, "until answered");
        assert!(board.dismiss_question(&id(2)));
        assert!(!board.dismiss_question(&id(2)));
        assert_eq!(board.tasks[1].phase, Phase::NeedsInput, "still waits");
        // The answer arrives as a follow-up in the same conversation.
        board.apply(&started(1, Origin::Background, "Finance"), None, t0);
        assert_eq!(board.tasks[0].phase, Phase::Running);
        assert!(board.tasks[0].question.is_none());

        let step = |title: &str, done| PlanStep {
            title: title.into(),
            done,
        };
        board.apply(
            &Event::Plan {
                conversation: id(1),
                steps: vec![step("a", true), step("b", false), step("c", false)],
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].progress(), Some((1, 3)));
        assert_eq!(board.tasks[0].current_step(), Some(1));
    }

    #[test]
    fn an_escalated_here_question_joins_the_background_tasks() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&quick(1), None, t0);
        assert!(board.stack().is_empty());
        assert!(board.apply(
            &Event::Escalated {
                conversation: id(1)
            },
            None,
            t0
        ));
        assert_eq!(board.stack().len(), 1);
        assert_eq!(board.pops.items.len(), 1, "its title pops from the orb");
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(1), t0);
        Bubble::apply(
            &mut bubble,
            &Event::Escalated {
                conversation: id(1),
            },
            t0,
        );
        let b = bubble.as_mut().unwrap();
        assert!(b.escalated);
        assert_eq!(b.due(t0, false), Due::Leave, "it flies off at once");
    }

    #[test]
    fn seeded_tasks_keep_their_paused_and_waiting_states() {
        let status = |n: u32, state| TaskStatus {
            conversation: id(n),
            origin: Origin::Background,
            title: "t".into(),
            state,
            step: Some("Opened Mail".into()),
            windows: Vec::new(),
            colour: 9,
        };
        let board = Board::seeded(vec![
            status(1, TaskState::Paused),
            status(2, TaskState::Waiting { until_ms: 5 }),
        ]);
        assert_eq!(board.tasks[0].phase, Phase::Paused);
        assert_eq!(board.tasks[1].phase, Phase::Waiting { until_ms: 5 });
        assert_eq!(board.tasks[0].colour, 1);
        assert_eq!(board.tasks[0].steps, ["Opened Mail"]);
    }

    #[test]
    fn a_task_keeps_its_last_three_steps_and_knows_where_it_works() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "a"), None, t0);
        assert_eq!(board.tasks[0].place(), Place::Unknown);
        for step in [
            "Searched the web for Nopa",
            "Read the menu",
            "Read the menu",
            "Chose 8 pm",
            "Booked",
        ] {
            board.apply(
                &Event::Step {
                    conversation: id(1),
                    description: step.into(),
                },
                None,
                t0,
            );
        }
        assert_eq!(
            board.tasks[0].steps,
            ["Read the menu", "Chose 8 pm", "Booked"]
        );
        board.apply(&started(2, Origin::Background, "b"), None, t0);
        board.apply(
            &Event::Step {
                conversation: id(2),
                description: "Opened https://resy.com".into(),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[1].place(), Place::Web);
        board.apply(
            &Event::Window {
                conversation: id(2),
                window: window("Safari"),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[1].place(), Place::Web, "a browser's window");
        board.apply(
            &Event::Window {
                conversation: id(2),
                window: window("Numbers"),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[1].place(), Place::Desktop);
    }
}
