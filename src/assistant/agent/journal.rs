//! A job's journal, `journal.md` in its workspace: its memory across
//! sessions, restarts, and days. The agent writes what it learned, tried,
//! decided, and what is open (`rules`); Agent-Desk adds its own lines,
//! marked `hex:`, for what happens around the agent: the start and budget,
//! milestones (a plan step ticked), a step whose files are missing, a fresh
//! session, a resume after a restart, waiting for the network or a usage
//! limit. A fresh session (`rollover`) and a resumed job are seeded from it.
//!
//! Beside it, `.hex-journal.json` keeps what Agent-Desk needs exactly (the
//! start time, sessions, milestones recorded, the session's size, the pages
//! the browsers were on, notes for the next prompt).
//!
//! OWNER: brain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::workspace;

/// The journal's file in a workspace.
pub const JOURNAL: &str = "journal.md";
/// Agent-Desk's own record beside it.
pub const STATE: &str = ".hex-journal.json";

/// What a job is told about its journal, its plan's checks, its time, and
/// what to do when something breaks (appended to its rules), naming only
/// the tools it has: the desktop's (`desktop`) and the browser's
/// (`browser`).
pub fn rules(desktop: bool, browser: bool) -> String {
    let mut rules = String::from(
        "Long jobs: journal.md in the workspace is your memory across sessions and restarts; Agent-Desk adds the `hex:` lines and may start you in a fresh session seeded from it. Append (Edit, never rewrite) one line whenever you learn, decide, or try something: `- learned: …`, `- decided: …`, `- open: …`, `- tried: <step> · <method> · failed: <why>`. After each step run its check before ticking it. When a check fails, log it as `tried:` and switch strategy (another method, tool, or a helper); never try the same method a third time. Agent-Desk tells you the time used and left: after each milestone leave a usable partial result (files, report.md); with little time left, secure what is done and answer with what is open. Tell the person nothing between milestones unless they asked for updates.",
    );
    if desktop {
        rules.push_str(" An app that stops responding: note it, quit it if you opened it, reopen it with hex_open_app, and go on.");
    }
    if browser {
        rules.push_str(" A browser session that died: open it again under the same name and go back to its page (the journal lists them).");
    }
    rules.push('\n');
    rules
}

/// What Agent-Desk notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Started,
    Milestone,
    CheckFailed,
    Session,
    Resumed,
    Waiting,
    Back,
    Failed,
    Checked,
    Done,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Milestone => "milestone",
            Self::CheckFailed => "check failed",
            Self::Session => "new session",
            Self::Resumed => "resumed",
            Self::Waiting => "waiting",
            Self::Back => "back",
            Self::Failed => "failed",
            Self::Checked => "checked",
            Self::Done => "done",
        }
    }
}

/// Agent-Desk's exact record of the job.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// When the job started (Unix ms).
    pub started_ms: u64,
    /// Its time budget in minutes; 0 has none.
    pub budget_minutes: u32,
    /// Backend sessions so far, the first included.
    pub sessions: u32,
    /// Plan steps recorded as done, in order.
    pub milestones: Vec<String>,
    /// The current session's last known size, in tokens.
    pub context_tokens: u64,
    /// Each browser session's last page.
    pub pages: BTreeMap<String, String>,
    /// What the next prompt must say (a ticked step whose files are
    /// missing).
    pub pending: Vec<String>,
}

/// A job's journal.
#[derive(Debug)]
pub struct Journal {
    dir: PathBuf,
    pub state: State,
}

impl Journal {
    /// The journal in `workspace`, started now for `request` when the job
    /// has none yet.
    pub fn open(workspace: &Path, now_ms: u64, request: &str, budget_minutes: u32) -> Self {
        let state = std::fs::read(workspace.join(STATE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<State>(&bytes).ok());
        let fresh = state.is_none();
        let mut journal = Self {
            dir: workspace.to_path_buf(),
            state: state.unwrap_or(State {
                started_ms: now_ms,
                budget_minutes,
                sessions: 1,
                ..State::default()
            }),
        };
        if fresh {
            if !workspace.join(JOURNAL).exists() {
                let _ = std::fs::write(
                    workspace.join(JOURNAL),
                    "# Journal\n\nThe job's memory across sessions and restarts. Agent-Desk adds the `hex:` lines; the job adds what it learned, tried, decided, and what is open.\n\n",
                );
            }
            let budget = if budget_minutes == 0 {
                "no time limit".to_owned()
            } else {
                format!("budget {}", span(u64::from(budget_minutes) * 60_000))
            };
            journal.note(
                now_ms,
                Kind::Started,
                &format!(
                    "{budget} · “{}”",
                    crate::assistant::store::shorten(request.trim(), 200)
                ),
            );
        }
        journal
    }

    /// Adds Agent-Desk's line `kind: text`.
    pub fn note(&mut self, now_ms: u64, kind: Kind, text: &str) {
        let line = format!(
            "- {} hex: {}: {}\n",
            stamp(now_ms),
            kind.word(),
            text.trim().replace('\n', " ")
        );
        let added = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(JOURNAL))
            .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
        if let Err(error) = added {
            tracing::debug!(%error, "couldn't add to the job's journal");
        }
        self.save();
    }

    /// Notes each plan step ticked since last time as a milestone, once,
    /// and checks the files its criterion names: a ticked step whose files
    /// are missing is noted and goes into the next prompt. The new
    /// milestones.
    pub fn milestones(&mut self, now_ms: u64) -> Vec<String> {
        let mut new = Vec::new();
        for step in workspace::steps(&self.dir) {
            if !step.done || self.state.milestones.contains(&step.title) {
                continue;
            }
            self.state.milestones.push(step.title.clone());
            self.note(now_ms, Kind::Milestone, &step.title);
            if let Some(missing) = step
                .check
                .as_deref()
                .and_then(|check| workspace::missing_files(&self.dir, check))
                .filter(|missing| !missing.is_empty())
            {
                let why = format!(
                    "“{}” is ticked, but its check names files that aren't there: {}",
                    step.title,
                    missing.join(", ")
                );
                self.note(now_ms, Kind::CheckFailed, &why);
                self.state.pending.push(why);
            }
            new.push(step.title);
        }
        if !new.is_empty() {
            self.save();
        }
        new
    }

    /// Notes a fresh session, for `why`; its number.
    pub fn new_session(&mut self, now_ms: u64, why: &str) -> u32 {
        self.state.sessions += 1;
        self.state.context_tokens = 0;
        let number = self.state.sessions;
        self.note(now_ms, Kind::Session, &format!("#{number}, {why}"));
        number
    }

    /// The current session's size now.
    pub fn sized(&mut self, tokens: u64) {
        if self.state.context_tokens != tokens {
            self.state.context_tokens = tokens;
            self.save();
        }
    }

    /// The page browser `session` is on.
    pub fn page(&mut self, session: &str, url: &str) {
        if self.state.pages.get(session).map(String::as_str) != Some(url) {
            self.state.pages.insert(session.to_owned(), url.to_owned());
            self.save();
        }
    }

    /// The notes the next prompt must carry; they are said once.
    pub fn take_pending(&mut self) -> Vec<String> {
        let pending = std::mem::take(&mut self.state.pending);
        if !pending.is_empty() {
            self.save();
        }
        pending
    }

    /// The journal's text.
    pub fn text(&self) -> String {
        std::fs::read_to_string(self.dir.join(JOURNAL)).unwrap_or_default()
    }

    /// How long the job has run at `now_ms`, of how much, as a line for its
    /// prompt.
    pub fn time_line(&self, now_ms: u64) -> String {
        let used = now_ms.saturating_sub(self.state.started_ms);
        if self.state.budget_minutes == 0 {
            return format!("Time: running for {}, no time limit.", span(used));
        }
        let budget = u64::from(self.state.budget_minutes) * 60_000;
        format!(
            "Time: running for {} of a {} budget; {} left.",
            span(used),
            span(budget),
            span(budget.saturating_sub(used))
        )
    }

    /// Whether less than a tenth of the budget is left at `now_ms`.
    pub fn nearly_out(&self, now_ms: u64) -> bool {
        let budget = u64::from(self.state.budget_minutes) * 60_000;
        budget > 0 && now_ms.saturating_sub(self.state.started_ms) * 10 >= budget * 9
    }

    fn save(&self) {
        let Ok(bytes) = serde_json::to_vec_pretty(&self.state) else {
            return;
        };
        if let Err(error) = std::fs::write(self.dir.join(STATE), bytes) {
            tracing::debug!(%error, "couldn't keep the job's journal state");
        }
    }
}

/// The newest part of `text` within `max` characters, whole lines, with a
/// note when older lines were left out.
pub fn digest(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.trim_start().starts_with(['-', '*']))
        .collect();
    let mut kept = Vec::new();
    let mut size = 0;
    for line in lines.iter().rev() {
        size += line.len() + 1;
        if size > max {
            break;
        }
        kept.push(*line);
    }
    let left_out = lines.len() - kept.len();
    kept.reverse();
    let mut digest = kept.join("\n");
    if left_out > 0 {
        digest = format!("({left_out} older lines are in journal.md)\n{digest}");
    }
    digest
}

/// The methods that failed, newest last: the agent's `tried:` lines and
/// Agent-Desk's failed checks, at most `max`.
pub fn failures(text: &str, max: usize) -> Vec<String> {
    let all: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let line = line.trim_start().strip_prefix(['-', '*'])?.trim();
            if let Some(tried) = line.strip_prefix("tried:") {
                return Some(tried.trim().to_owned());
            }
            let (_, rest) = line.split_once(" hex: ")?;
            rest.strip_prefix("check failed:")
                .map(|why| why.trim().to_owned())
        })
        .collect();
    all[all.len().saturating_sub(max)..].to_vec()
}

/// What the agent left open (`- open:` lines).
pub fn open_items(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            line.trim_start()
                .strip_prefix(['-', '*'])?
                .trim()
                .strip_prefix("open:")
                .map(|open| open.trim().to_owned())
        })
        .filter(|open| !open.is_empty())
        .collect()
}

/// A duration as people say it: "45 min", "3 h 10 min", "2 days 4 h".
pub fn span(ms: u64) -> String {
    let minutes = ms / 60_000;
    let (days, hours, minutes) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (days, hours, minutes) {
        (0, 0, m) => format!("{m} min"),
        (0, h, 0) => format!("{h} h"),
        (0, h, m) => format!("{h} h {m} min"),
        (1, 0, _) => "1 day".into(),
        (d, 0, _) => format!("{d} days"),
        (1, h, _) => format!("1 day {h} h"),
        (d, h, _) => format!("{d} days {h} h"),
    }
}

/// `now_ms` in local time, `YYYY-MM-DD HH:MM`.
fn stamp(now_ms: u64) -> String {
    match crate::assistant::schedules::local_tm(now_ms) {
        Some(tm) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min
        ),
        None => format!("t+{}s", now_ms / 1000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hex-journal-{name}-{}-{}",
            std::process::id(),
            crate::assistant::store::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_journal_starts_once_and_notes_each_milestone_once() {
        let dir = folder("start");
        let start = 1_794_472_200_000;
        let mut journal = Journal::open(&dir, start, "Research 50 suppliers", 600);
        // Opened again (a resume): no second start.
        let mut journal2 = Journal::open(&dir, start + 1000, "Research 50 suppliers", 600);
        assert_eq!(journal2.state.started_ms, start);
        std::fs::write(
            dir.join(workspace::PLAN),
            "- [x] Find suppliers (browser) — check: `suppliers.csv` exists\n- [ ] Write the sheet — check: `sheet.xlsx`\n",
        )
        .unwrap();
        assert_eq!(
            journal2.milestones(start + 60_000),
            ["Find suppliers (browser)"]
        );
        assert!(
            journal2.milestones(start + 120_000).is_empty(),
            "noted once"
        );
        // The ticked step's file is missing: noted, and said next time.
        let pending = journal2.take_pending();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].contains("suppliers.csv"), "{pending:?}");
        assert!(journal2.take_pending().is_empty());
        let text = journal2.text();
        assert_eq!(text.matches("hex: started:").count(), 1, "{text}");
        assert!(
            text.contains("budget 10 h · “Research 50 suppliers”"),
            "{text}"
        );
        assert_eq!(text.matches("hex: milestone: Find suppliers").count(), 1);
        assert!(text.contains("hex: check failed:"));
        assert_eq!(failures(&text, 5).len(), 1);
        journal.state = Journal::open(&dir, start, "", 0).state;
        assert_eq!(journal.state.milestones, ["Find suppliers (browser)"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_job_knows_its_time_and_when_it_is_nearly_out() {
        let dir = folder("time");
        let journal = Journal::open(&dir, 0, "x", 600);
        assert_eq!(
            journal.time_line(3 * 3_600_000 + 600_000),
            "Time: running for 3 h 10 min of a 10 h budget; 6 h 50 min left."
        );
        assert!(!journal.nearly_out(8 * 3_600_000));
        assert!(journal.nearly_out(9 * 3_600_000));
        assert_eq!(span(2 * 86_400_000 + 4 * 3_600_000), "2 days 4 h");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_agent_s_own_lines_are_read_back() {
        let text = "# Journal\n\n- 2026-11-13 10:00 hex: started: budget 10 h\n- learned: the shop lists prices without VAT\n- tried: Find suppliers · Google search · failed: captcha\n- open: which currency?\n- tried: Find suppliers · Bing · failed: no results\n";
        assert_eq!(
            failures(text, 1),
            ["Find suppliers · Bing · failed: no results"]
        );
        assert_eq!(open_items(text), ["which currency?"]);
        let short = digest(text, 120);
        assert!(short.starts_with("("), "{short}");
        assert!(short.ends_with("no results"), "{short}");
        assert!(!digest(text, 10_000).starts_with('('));
    }
}
