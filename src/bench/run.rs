//! Runs tasks against an [`App`] (the running Agent-Desk, or a fake in tests):
//! asks each one, follows its conversation until it ends, reads the desktop
//! read-only, and checks the result. The app, the desktop, and the clock
//! are injected, so the whole loop runs in unit tests without Agent-Desk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use color_eyre::{Result, eyre::eyre};

use super::check::{self, Observation, ToolCall, Verdict, Win};
use super::results::{Status, TaskResult};
use super::task::{Check, Config, Front, Lang, Task, fill};

/// One conversation as the app saved it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conv {
    /// `answered`, `failed`, `cancelled`, `interrupted`, or `running`.
    pub outcome: String,
    /// `action_shortcut`, `background`, or `chat`.
    pub origin: String,
    /// When the assistant's messages came, any kind (Unix ms).
    pub replies: Vec<u64>,
    /// The assistant's text messages: (Unix ms, text).
    pub answers: Vec<(u64, String)>,
    pub actions: Vec<String>,
    pub workspace: Option<PathBuf>,
}

impl Conv {
    pub fn running(&self) -> bool {
        self.outcome == "running"
    }
}

/// The running Agent-Desk, as the bench uses it.
pub trait App {
    /// The app's version (`agent-desk status`).
    fn version(&mut self) -> Result<String>;
    /// Asks `text` (continuing `conversation`), as the overlay would with
    /// no window in front; the conversation's id.
    fn submit(&mut self, text: &str, conversation: Option<&str>) -> Result<String>;
    fn conversation(&mut self, id: &str) -> Option<Conv>;
    /// Conversations created at or after `since_ms` (jobs a turn started).
    fn created_since(&mut self, since_ms: u64) -> Vec<String>;
    fn tool_calls(&mut self, id: &str) -> Vec<ToolCall>;
    /// Asks the app to stop the task running `id`.
    fn stop(&mut self, id: &str);
}

/// The desktop, read only.
pub trait Desktop {
    fn front_app(&mut self) -> Option<String>;
    fn apps(&mut self) -> Option<Vec<String>>;
    fn windows(&mut self, app: &str) -> Option<Vec<Win>>;
}

pub trait Clock {
    fn now_ms(&self) -> u64;
    fn sleep_ms(&self, ms: u64);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        super::now_ms()
    }

    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

pub struct Options {
    pub langs: Vec<Lang>,
    pub allow_front: bool,
    pub poll_ms: u64,
    /// `~/Documents/Agent-Desk/bench/<run>`: every task's folder is inside it.
    pub run_dir: PathBuf,
    pub run_id: String,
    /// How long to wait for a background job to show it runs.
    pub background_wait_ms: u64,
}

/// What a run keeps between tasks.
#[derive(Default)]
struct Memory {
    /// Each task's conversation, by (id, language), for `after`.
    conversations: BTreeMap<(String, Lang), String>,
}

/// Runs `tasks` in order, each in every language of `options`, reporting
/// each result to `progress` as it ends.
pub fn run(
    tasks: &[Task],
    config: &Config,
    app: &mut dyn App,
    desktop: &mut dyn Desktop,
    clock: &dyn Clock,
    options: &Options,
    progress: &mut dyn FnMut(&TaskResult),
) -> Vec<TaskResult> {
    let mut memory = Memory::default();
    let mut results = Vec::new();
    for task in tasks {
        for &lang in &options.langs {
            let result = run_one(
                task,
                lang,
                config,
                app,
                desktop,
                clock,
                options,
                &mut memory,
            );
            progress(&result);
            results.push(result);
        }
    }
    results
}

fn skipped(task: &Task, lang: Lang, reason: impl Into<String>) -> TaskResult {
    TaskResult::new(&task.id, lang, Status::Skipped, reason)
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    task: &Task,
    lang: Lang,
    config: &Config,
    app: &mut dyn App,
    desktop: &mut dyn Desktop,
    clock: &dyn Clock,
    options: &Options,
    memory: &mut Memory,
) -> TaskResult {
    let missing = task.missing(config);
    if !missing.is_empty() {
        return skipped(
            task,
            lang,
            format!("bench/config.toml lacks {}", missing.join(", ")),
        );
    }
    if task.safety.front == Front::MayChange && !options.allow_front {
        return skipped(
            task,
            lang,
            "it brings an app forward; run with --allow-front",
        );
    }
    let continued = match &task.after {
        Some(after) => match memory.conversations.get(&(after.clone(), lang)) {
            Some(id) => Some(id.clone()),
            None => return skipped(task, lang, format!("{after} didn't run")),
        },
        None => None,
    };
    let temp_dir = options.run_dir.join(format!("{}-{}", task.id, lang.code()));
    let mut values: BTreeMap<String, String> = config.clone();
    values.insert("temp_dir".into(), temp_dir.display().to_string());
    values.insert("run".into(), options.run_id.clone());
    if let Err(error) = prepare_dir(&temp_dir, task, &values) {
        return TaskResult::new(&task.id, lang, Status::Error, format!("{error:#}"));
    }
    let say = |text: &str| fill(text, &values, false);
    let Ok(text) = say(task.text.get(lang)) else {
        return skipped(task, lang, "a placeholder has no value");
    };

    // Preconditions: read the desktop only.
    if !task.precondition.is_empty() {
        let mut before = Observation {
            lang: Some(lang),
            temp_dir: temp_dir.clone(),
            front_after: desktop.front_app(),
            apps_after: desktop.apps(),
            ..Observation::default()
        };
        before.windows_after = read_windows(desktop, &task.precondition, &values);
        if let Some(failed) = task
            .precondition
            .iter()
            .map(|check| check::evaluate(check, &before, &values))
            .find(|verdict| !verdict.passed)
        {
            return skipped(
                task,
                lang,
                format!(
                    "precondition {} doesn't hold ({})",
                    failed.check, failed.detail
                ),
            );
        }
    }

    let front_before = desktop.front_app();
    let mut fronts = Vec::new();
    let mut own = Vec::new();

    // A background job first, for “answer while a job runs”.
    let background = match &task.background {
        Some(job) => {
            let text = match say(match lang {
                Lang::De => &job.de,
                Lang::En => &job.en,
            }) {
                Ok(text) => text,
                Err(_) => return skipped(task, lang, "a placeholder has no value"),
            };
            let started = clock.now_ms();
            let id = match app.submit(&text, None) {
                Ok(id) => id,
                Err(error) => {
                    return TaskResult::new(&task.id, lang, Status::Error, format!("{error:#}"));
                }
            };
            own.push(id.clone());
            // Until it runs and its lead has passed.
            loop {
                let now = clock.now_ms();
                let runs = app.conversation(&id).is_some_and(|conv| conv.running());
                if (runs && now >= started + job.lead_ms)
                    || now >= started + job.lead_ms.max(options.background_wait_ms)
                {
                    break;
                }
                clock.sleep_ms(options.poll_ms);
            }
            Some(id)
        }
        None => None,
    };

    let asked_ms = clock.now_ms();
    let id = match app.submit(&text, continued.as_deref()) {
        Ok(id) => id,
        Err(error) => {
            if let Some(job) = &background {
                app.stop(job);
            }
            return TaskResult::new(&task.id, lang, Status::Error, format!("{error:#}"));
        }
    };
    own.push(id.clone());
    let deadline = asked_ms + task.timeout_s * 1000;
    let mut first_answer_ms = None;
    let mut background_running_at_answer = None;
    let mut seen_start = false;
    let mut children: Vec<String> = Vec::new();
    let mut conv = Conv::default();
    let timed_out = loop {
        let now = clock.now_ms();
        if let Some(front) = desktop.front_app()
            && fronts.last() != Some(&front)
        {
            fronts.push(front);
        }
        if let Some(current) = app.conversation(&id) {
            // A continued conversation holds older messages and still shows
            // the last outcome until the new request runs: only running, or
            // a reply after asking, says this request started.
            seen_start |= current.running() || current.replies.iter().any(|&at| at >= asked_ms);
            if first_answer_ms.is_none()
                && let Some((at, _)) = current.answers.iter().find(|(at, _)| *at >= asked_ms)
            {
                first_answer_ms = Some(at.saturating_sub(asked_ms).min(now - asked_ms));
                if let Some(job) = &background {
                    background_running_at_answer =
                        Some(app.conversation(job).is_some_and(|job| job.running()));
                }
            }
            conv = current;
        }
        for child in app.created_since(asked_ms) {
            if !own.contains(&child) && !children.contains(&child) {
                children.push(child);
            }
        }
        let children_run = children
            .iter()
            .any(|child| app.conversation(child).is_some_and(|conv| conv.running()));
        let main_done = seen_start && !conv.running() && !conv.outcome.is_empty();
        if main_done && !children_run {
            break false;
        }
        if now >= deadline {
            break true;
        }
        clock.sleep_ms(options.poll_ms);
    };
    let finished_ms = clock.now_ms();
    if timed_out {
        app.stop(&id);
        for child in &children {
            app.stop(child);
        }
    }
    if let Some(job) = &background {
        app.stop(job);
    }
    memory
        .conversations
        .insert((task.id.clone(), lang), id.clone());

    // What it did, and what the desktop looks like now.
    let mut tools = app.tool_calls(&id);
    let mut actions = conv.actions.clone();
    let mut workspace = conv.workspace.clone();
    for child in &children {
        tools.extend(app.tool_calls(child));
        if let Some(child) = app.conversation(child) {
            actions.extend(child.actions);
            workspace = workspace.or(child.workspace);
        }
    }
    let answer = conv
        .answers
        .iter()
        .filter(|(at, _)| *at >= asked_ms)
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let plan = workspace
        .as_deref()
        .map(|dir| read_plan(&dir.join("plan.md")))
        .unwrap_or_default();
    let mut seen = Observation {
        lang: Some(lang),
        outcome: if timed_out {
            "timeout".into()
        } else {
            conv.outcome.clone()
        },
        answer,
        first_answer_ms,
        total_ms: finished_ms.saturating_sub(asked_ms),
        tools,
        actions,
        job: workspace.is_some() || conv.origin == "background" || !children.is_empty(),
        workspace,
        plan,
        temp_dir,
        front_before,
        fronts,
        front_after: desktop.front_app(),
        apps_after: desktop.apps(),
        windows_after: BTreeMap::new(),
        background_running_at_answer,
        continued,
        conversation: Some(id.clone()),
    };
    seen.windows_after = read_windows(desktop, &task.checks, &values);

    let mut verdicts: Vec<Verdict> = Vec::new();
    verdicts.extend(check::lane(task.lane, &seen));
    if task.safety.front == Front::Keep {
        verdicts.push(check::front_kept(&seen));
    }
    verdicts.extend(
        task.checks
            .iter()
            .map(|check| check::evaluate(check, &seen, &values)),
    );
    let status = if timed_out {
        Status::Timeout
    } else if verdicts.iter().all(|verdict| verdict.passed) {
        Status::Passed
    } else {
        Status::Failed
    };
    let reason = verdicts
        .iter()
        .filter(|verdict| !verdict.passed)
        .map(|verdict| match verdict.detail.is_empty() {
            true => verdict.check.clone(),
            false => format!("{} ({})", verdict.check, verdict.detail),
        })
        .collect::<Vec<_>>()
        .join("; ");
    TaskResult {
        id: task.id.clone(),
        lang,
        status,
        reason,
        conversation: Some(id),
        first_answer_ms,
        total_ms: Some(seen.total_ms),
        steps: Some(seen.steps()),
        lane: Some(if seen.job { "job" } else { "quick" }.into()),
        checks: verdicts,
        answer: super::shorten(&seen.answer, 400),
    }
}

/// Creates the task's temp folder, empty, with its seed files.
fn prepare_dir(dir: &Path, task: &Task, values: &BTreeMap<String, String>) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    for seed in &task.seed {
        let name =
            fill(&seed.name, values, false).map_err(|key| eyre!("no value for {{{key}}}"))?;
        let content = fill(&seed.content, values, false).unwrap_or_else(|_| seed.content.clone());
        std::fs::write(dir.join(name), content)?;
    }
    Ok(())
}

/// The windows of every app `checks` name.
fn read_windows(
    desktop: &mut dyn Desktop,
    checks: &[Check],
    values: &BTreeMap<String, String>,
) -> BTreeMap<String, Vec<Win>> {
    let mut windows = BTreeMap::new();
    for check in checks {
        if let Check::Window { app, .. } = check {
            let app = fill(app, values, false).unwrap_or_else(|_| app.clone());
            if !windows.contains_key(&app)
                && let Some(found) = desktop.windows(&app)
            {
                windows.insert(app, found);
            }
        }
    }
    windows
}

/// A job's `plan.md` steps: `- [ ]` and `- [x]` lines.
pub fn read_plan(path: &Path) -> Vec<(bool, String)> {
    parse_plan(&std::fs::read_to_string(path).unwrap_or_default())
}

pub fn parse_plan(text: &str) -> Vec<(bool, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start().strip_prefix(['-', '*'])?.trim_start();
            let (done, rest) = if let Some(rest) = line.strip_prefix("[ ]") {
                (false, rest)
            } else {
                (
                    true,
                    line.strip_prefix("[x]")
                        .or_else(|| line.strip_prefix("[X]"))?,
                )
            };
            let title = rest.trim();
            (!title.is_empty()).then(|| (done, title.to_owned()))
        })
        .collect()
}
