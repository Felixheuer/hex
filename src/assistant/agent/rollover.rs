//! A long job's backend session grows with every step: past a point it is
//! slow, expensive, and the backend starts compacting it on its own,
//! losing detail. Agent-Desk measures it (`Meter`: the session's size in
//! tokens from the backend's own reports, its steps, and how long a step
//! takes) and, between two steps with no tool running, ends it
//! (`Control::rotate`) and goes on in a fresh session seeded with the
//! request, the plan, the journal, what is open, what failed, and the time
//! (`seed`). Nothing is lost: the work is in the workspace, and the journal
//! remembers the rest.
//!
//! OWNER: brain.

use std::collections::VecDeque;
use std::path::Path;

use super::journal::{self, Journal};
use super::{Job, workspace};

/// What a backend reports of its session after a step (`Report::Usage`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// The session's size: the tokens its last request carried (input,
    /// cached, and written to the cache).
    pub context_tokens: u64,
}

/// When a session is due for a fresh one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Its size, under the 200k window and before the backend's own
    /// compaction.
    pub context_tokens: u64,
    /// Its steps.
    pub steps: u32,
    /// A step taking this many times as long as the session's first ones…
    pub slow_factor: u64,
    /// …once it has had at least this many steps.
    pub slow_after: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            context_tokens: 140_000,
            steps: 250,
            slow_factor: 3,
            slow_after: 40,
        }
    }
}

/// Fresh sessions one run may start before it stops measuring (a backend
/// that reports a huge size from its first step would roll forever).
pub const MAX_ROLLOVERS: u32 = 60;

/// Steps averaged for "how long a step takes".
const WINDOW: usize = 8;

/// Why a session is due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    Large(u64),
    Long(u32),
    Slow { now_ms: u64, first_ms: u64 },
}

impl Why {
    /// For the journal: "the session reached 152k tokens".
    pub fn words(&self) -> String {
        match self {
            Self::Large(tokens) => format!("the session reached {}k tokens", tokens / 1000),
            Self::Long(steps) => format!("the session took {steps} steps"),
            Self::Slow { now_ms, first_ms } => format!(
                "a step took {} s, against {} s at the start",
                now_ms / 1000,
                first_ms / 1000
            ),
        }
    }
}

/// One session's measure.
#[derive(Clone, Debug, Default)]
pub struct Meter {
    limits: Limits,
    steps: u32,
    tokens: u64,
    last_ms: Option<u64>,
    first: Vec<u64>,
    recent: VecDeque<u64>,
}

impl Meter {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// Starts measuring at `now_ms` (a run starts or goes on).
    pub fn start(&mut self, now_ms: u64) {
        self.last_ms = Some(now_ms);
    }

    /// The session's size now.
    pub fn tokens(&self) -> u64 {
        self.tokens
    }

    /// A step ended at `now_ms` with the session at `usage`: whether the
    /// session is due for a fresh one, and why.
    pub fn step(&mut self, usage: Usage, now_ms: u64) -> Option<Why> {
        self.steps += 1;
        self.tokens = usage.context_tokens;
        if let Some(last) = self.last_ms.replace(now_ms) {
            let took = now_ms.saturating_sub(last);
            if self.first.len() < WINDOW {
                self.first.push(took);
            } else {
                self.recent.push_back(took);
                if self.recent.len() > WINDOW {
                    self.recent.pop_front();
                }
            }
        }
        if self.tokens >= self.limits.context_tokens {
            return Some(Why::Large(self.tokens));
        }
        if self.steps >= self.limits.steps {
            return Some(Why::Long(self.steps));
        }
        let average = |times: &[u64]| times.iter().sum::<u64>() / times.len().max(1) as u64;
        let first = average(&self.first);
        let recent = average(self.recent.make_contiguous());
        if self.steps >= self.limits.slow_after
            && self.recent.len() == WINDOW
            && first > 0
            && recent >= first * self.limits.slow_factor
        {
            return Some(Why::Slow {
                now_ms: recent,
                first_ms: first,
            });
        }
        None
    }
}

/// Characters of the journal a seed carries.
const JOURNAL_CHARS: usize = 6_000;
/// Characters of the plan a seed carries.
const PLAN_CHARS: usize = 6_000;

/// What a run that goes on after a gap is told beside its own note: the
/// time, the plan and journal, what is open, what failed, the pages its
/// browsers were on, and what Agent-Desk found (`pending`).
pub fn context(workspace: &Path, journal: &Journal, now_ms: u64) -> String {
    let mut text = format!("{}\n", journal.time_line(now_ms));
    if journal.nearly_out(now_ms) {
        text.push_str(NEARLY_OUT);
        text.push('\n');
    }
    let plan = std::fs::read_to_string(workspace.join(workspace::PLAN)).unwrap_or_default();
    if !plan.trim().is_empty() {
        text.push_str(&format!(
            "\n{} now:\n<<<\n{}\n>>>\n",
            workspace::PLAN,
            lines_within(plan.trim(), PLAN_CHARS)
        ));
    }
    let log = journal.text();
    let digest = journal::digest(&log, JOURNAL_CHARS);
    if !digest.trim().is_empty() {
        text.push_str(&format!(
            "\n{} (your memory, newest last):\n<<<\n{digest}\n>>>\n",
            journal::JOURNAL
        ));
    }
    let open = journal::open_items(&log);
    if !open.is_empty() {
        text.push_str(&format!("\nStill open: {}\n", open.join("; ")));
    }
    let failed = journal::failures(&log, 8);
    if !failed.is_empty() {
        text.push_str(&format!(
            "\nAlready failed, so use another method, tool, or a helper for these: {}\n",
            failed.join("; ")
        ));
    }
    for pending in &journal.state.pending {
        text.push_str(&format!(
            "\nAgent-Desk found: {pending}. Do that step again another way.\n"
        ));
    }
    if !journal.state.pages.is_empty() {
        let pages: Vec<String> = journal
            .state
            .pages
            .iter()
            .map(|(session, url)| format!("{session}: {url}"))
            .collect();
        text.push_str(&format!(
            "\nYour browsers were on these pages (reopen what you still need): {}\n",
            pages.join(", ")
        ));
    }
    text
}

/// The first whole lines of `text` within `max` characters (a plan keeps
/// its checklist lines as they are).
fn lines_within(text: &str, max: usize) -> String {
    let mut kept = String::new();
    for line in text.lines() {
        if kept.len() + line.len() + 1 > max {
            kept.push_str("… (the rest is in the file)\n");
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept.trim_end().to_owned()
}

/// What a job with little time left is told.
pub const NEARLY_OUT: &str = "Less than a tenth of your time is left: finish the step in hand, secure what is done (files, report.md), and answer with what is done and what is still open.";

/// The first message of a fresh session that continues `job` after `why`.
pub fn seed(job: &Job, workspace: &Path, journal: &Journal, why: &str, now_ms: u64) -> String {
    format!(
        "{rules}\n\nYou are continuing a long job in a fresh session: {why}, so Agent-Desk started a new one. Nothing is lost: your workspace ({ws}) holds the files, {plan}, {report}, and {journal}. Ticked steps (`- [x]`) are done: never redo them, and never repeat a send or submit the journal or report says happened. Go on with the first open step, keep {plan} and {journal} up to date, and answer as usual when everything is done.\n\nThe job, as the person asked for it: {request}\n{context}",
        rules = super::request_rules(job),
        ws = workspace.display(),
        plan = workspace::PLAN,
        report = workspace::REPORT,
        journal = journal::JOURNAL,
        request = job.request.text.trim(),
        context = context(workspace, journal, now_ms),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_due_when_large_long_or_slow() {
        let mut meter = Meter::new(Limits::default());
        meter.start(0);
        assert_eq!(
            meter.step(
                Usage {
                    context_tokens: 30_000
                },
                1_000
            ),
            None
        );
        assert_eq!(
            meter.step(
                Usage {
                    context_tokens: 141_000
                },
                2_000
            ),
            Some(Why::Large(141_000))
        );
        let mut meter = Meter::new(Limits {
            steps: 3,
            ..Limits::default()
        });
        meter.start(0);
        meter.step(Usage::default(), 1);
        meter.step(Usage::default(), 2);
        assert_eq!(meter.step(Usage::default(), 3), Some(Why::Long(3)));
        // Steps of 10 s, then of 40 s: slow, once enough of them were.
        let mut meter = Meter::new(Limits {
            slow_after: 10,
            ..Limits::default()
        });
        meter.start(0);
        let mut now = 0;
        let mut due = None;
        for step in 0..30 {
            now += if step < 8 { 10_000 } else { 40_000 };
            due = meter.step(Usage::default(), now).or(due);
            if due.is_some() {
                break;
            }
        }
        assert_eq!(
            due,
            Some(Why::Slow {
                now_ms: 40_000,
                first_ms: 10_000
            })
        );
        assert!(due.unwrap().words().contains("40 s"));
    }
}
