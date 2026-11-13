//! One task as every surface reads it: where it is in its life, what it
//! does now, what it asked, its plan, and where it works.

use std::time::{Duration, Instant};

use crate::assistant::{ConversationId, Origin, PlanStep, TaskState, TaskWindow};

use super::text::{first_line, shorten};

/// How long a finished task stays listed with its ✓ or ✗.
pub const FINISHED_LINGER: Duration = Duration::from_secs(5);
/// A stopped task leaves sooner: the person stopped it and knows.
pub const STOPPED_LINGER: Duration = Duration::from_millis(1500);
/// How many of a task's latest steps a compact row shows.
pub const TASK_STEPS: usize = 3;
/// The most lines a task's log keeps while it runs; the stored
/// conversation has them all.
pub const LOG_MAX: usize = 200;
/// How long the screen frame stays after a task last acted on the screen.
pub const ACTING_FOR: Duration = Duration::from_secs(4);

/// Where a task is in its life.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
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
    /// Still running, queued, or held: not over.
    pub fn live(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Paused | Self::Waiting { .. } | Self::NeedsInput
        )
    }

    pub fn of(state: TaskState) -> Self {
        match state {
            TaskState::Queued => Self::Queued,
            TaskState::Running => Self::Running,
            TaskState::Paused => Self::Paused,
            TaskState::Waiting { until_ms } => Self::Waiting { until_ms },
            TaskState::NeedsInput => Self::NeedsInput,
        }
    }

    /// A waiting task that thinks or steps again is running; a paused one
    /// stays paused until it's handed back, and one asking until answered.
    pub(super) fn resumed(self) -> Self {
        match self {
            Self::Queued | Self::Waiting { .. } => Self::Running,
            other => other,
        }
    }
}

/// What a running task does right now, for its one status line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Activity {
    /// Waiting on the model, nothing done yet.
    #[default]
    Thinking,
    /// Doing things (its steps list).
    Working,
    /// Its answer streams in.
    Answering,
}

/// Why a live task isn't moving: the person has it, its agent hit a usage
/// limit, or it waits on the person's answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Hold {
    /// Taken over: its desktop actions wait until it's handed back.
    Paused,
    /// Retries at `until_ms` (Unix ms), and why.
    Waiting { until_ms: u64, reason: String },
    /// It asked the person something and waits for the answer.
    Asking(String),
}

impl Hold {
    /// The hold a task's listed state says, when nothing more is known
    /// (no reason, no question's words).
    pub fn of(state: TaskState) -> Option<Self> {
        match state {
            TaskState::NeedsInput => Some(Self::Asking(String::new())),
            TaskState::Paused => Some(Self::Paused),
            TaskState::Waiting { until_ms } => Some(Self::Waiting {
                until_ms,
                reason: String::new(),
            }),
            TaskState::Queued | TaskState::Running => None,
        }
    }
}

/// Where a task works, for the small glyph on its row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Place {
    /// On the web: a hidden browser session, or a browser's windows.
    Web,
    /// In apps' windows on the desktop.
    Desktop,
    /// Nothing yet says.
    Unknown,
}

/// One line of a running task's log, as it happened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Logged {
    /// A step, in the words it was reported in.
    Step(String),
    /// It started working in this app's window.
    Window(String),
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

/// One task as the surfaces read it.
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: ConversationId,
    pub origin: Origin,
    /// The question, shortened to a line.
    pub title: String,
    pub colour: u8,
    pub phase: Phase,
    /// What it does while it runs (thinking, working, answering).
    pub activity: Activity,
    /// The latest step, as a person reads it (shortened).
    pub step: Option<String>,
    /// The latest [`TASK_STEPS`] steps, shortened, oldest first.
    pub steps: Vec<String>,
    /// Every step and window of the run in flight, in full, up to
    /// [`LOG_MAX`]; a follow-up starts it afresh.
    pub log: Vec<Logged>,
    pub windows: Vec<TaskWindow>,
    /// The answer so far (streamed), or the final one.
    pub answer: String,
    /// Why it failed.
    pub error: Option<String>,
    /// Why it waits, while [`Phase::Waiting`] (the agent's usage limit).
    pub reason: String,
    /// It moves the real pointer or presses real keys now ("Hands off").
    pub hands_off: bool,
    /// What it asked the person and waits on, until answered or dismissed.
    pub question: Option<String>,
    /// When it asked, in the stream's order of questions (newest highest).
    pub asked: u64,
    /// Its plan, once it made one.
    pub plan: Vec<PlanStep>,
    /// A quick turn that moved to the background (`Event::Escalated`).
    pub escalated: bool,
    /// When it finished, for how long it lingers.
    pub(super) ended: Option<Instant>,
    /// When it started (or a follow-up started it again after it ended),
    /// as far as this view saw it: a job's card says how long it runs.
    pub since: Option<Instant>,
    /// The job it helps (`assistant::agent::team`): its card goes under
    /// that job's.
    pub lead: Option<ConversationId>,
    /// When it last worked on the screen (`Event::Acting`): the screen
    /// frame shows for [`ACTING_FOR`] after.
    pub acting_at: Option<Instant>,
}

impl Task {
    pub fn new(id: ConversationId, origin: Origin, title: &str, colour: u8, phase: Phase) -> Self {
        Self {
            id,
            origin,
            title: shorten(title),
            colour,
            phase,
            activity: Activity::Thinking,
            step: None,
            steps: Vec::new(),
            log: Vec::new(),
            windows: Vec::new(),
            answer: String::new(),
            error: None,
            reason: String::new(),
            hands_off: false,
            question: None,
            asked: 0,
            plan: Vec::new(),
            escalated: false,
            ended: None,
            since: None,
            lead: None,
            acting_at: None,
        }
    }

    /// A row's second line: the step while it runs, the answer's first line
    /// or the error once it's over.
    pub fn detail(&self) -> Option<String> {
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

    /// Why it isn't moving while it's live, as far as its events said.
    pub fn hold(&self) -> Option<Hold> {
        match self.phase {
            Phase::Paused => Some(Hold::Paused),
            Phase::Waiting { until_ms } => Some(Hold::Waiting {
                until_ms,
                reason: self.reason.clone(),
            }),
            Phase::NeedsInput => self.question.clone().map(Hold::Asking),
            _ => None,
        }
    }

    /// Where it works: the web when its windows are all browsers, or it has
    /// none and a step names the web; the desktop when it has windows.
    pub fn place(&self) -> Place {
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
    pub fn progress(&self) -> Option<(usize, usize)> {
        (!self.plan.is_empty()).then(|| {
            let done = self.plan.iter().filter(|step| step.done).count();
            (done, self.plan.len())
        })
    }

    /// The plan step it's on: the first not done.
    pub fn current_step(&self) -> Option<usize> {
        self.plan.iter().position(|step| !step.done)
    }

    pub(super) fn note_step(&mut self, description: &str) {
        let step = shorten(description);
        if self.steps.last() != Some(&step) {
            self.steps.push(step.clone());
            let extra = self.steps.len().saturating_sub(TASK_STEPS);
            self.steps.drain(..extra);
        }
        self.step = Some(step);
    }

    pub(super) fn log(&mut self, line: Logged) {
        if self.log.len() < LOG_MAX {
            self.log.push(line);
        }
    }

    pub(super) fn leaves_at(&self) -> Option<Instant> {
        let linger = if self.phase == Phase::Stopped {
            STOPPED_LINGER
        } else {
            FINISHED_LINGER
        };
        self.ended.map(|ended| ended + linger)
    }
}
