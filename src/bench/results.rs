//! A run's results (`bench/results/<version>-<timestamp>.json`) and the
//! comparison with the run before it.

use std::path::{Path, PathBuf};

use color_eyre::{Result, eyre::WrapErr};
use serde::{Deserialize, Serialize};

use super::check::Verdict;
use super::task::Lang;

pub const SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Timeout,
    /// The Mac isn't set up for it, or the run doesn't allow it.
    Skipped,
    /// The bench couldn't ask it (the app refused, a folder failed).
    Error,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Passed => "pass",
            Self::Failed => "FAIL",
            Self::Timeout => "TIMEOUT",
            Self::Skipped => "skip",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TaskResult {
    pub id: String,
    pub lang: Lang,
    pub status: Status,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default)]
    pub conversation: Option<String>,
    /// From asking to the first answer text.
    #[serde(default)]
    pub first_answer_ms: Option<u64>,
    /// From asking until it and every job it started ended.
    #[serde(default)]
    pub total_ms: Option<u64>,
    /// Tool calls (or steps the conversation shows, if more).
    #[serde(default)]
    pub steps: Option<usize>,
    /// `quick` or `job`, as it ran.
    #[serde(default)]
    pub lane: Option<String>,
    #[serde(default)]
    pub checks: Vec<Verdict>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer: String,
}

impl TaskResult {
    pub fn new(id: &str, lang: Lang, status: Status, reason: impl Into<String>) -> Self {
        Self {
            id: id.to_owned(),
            lang,
            status,
            reason: reason.into(),
            conversation: None,
            first_answer_ms: None,
            total_ms: None,
            steps: None,
            lane: None,
            checks: Vec::new(),
            answer: String::new(),
        }
    }

    fn key(&self) -> (String, Lang) {
        (self.id.clone(), self.lang)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RunResult {
    pub schema: u32,
    /// The app's version (`agent-desk status`).
    pub version: String,
    /// `dev` or `live`.
    pub target: String,
    pub run_id: String,
    pub started_ms: u64,
    pub finished_ms: u64,
    #[serde(default)]
    pub filter: Option<String>,
    pub tasks: Vec<TaskResult>,
}

impl RunResult {
    pub fn passed(&self) -> usize {
        self.count(Status::Passed)
    }

    /// Tasks that ran (not skipped).
    pub fn ran(&self) -> usize {
        self.tasks.len() - self.count(Status::Skipped)
    }

    fn count(&self, status: Status) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.status == status)
            .count()
    }

    fn median(&self, value: impl Fn(&TaskResult) -> Option<u64>) -> Option<u64> {
        let mut values: Vec<u64> = self
            .tasks
            .iter()
            .filter(|task| task.status != Status::Skipped)
            .filter_map(value)
            .collect();
        values.sort_unstable();
        values.get(values.len() / 2).copied()
    }

    pub fn file_name(&self) -> String {
        let version: String = self
            .version
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{version}-{}.json", super::timestamp(self.started_ms))
    }
}

/// Writes `result` into `dir`; its path.
pub fn save(dir: &Path, result: &RunResult) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("couldn't create {}", dir.display()))?;
    let path = dir.join(result.file_name());
    let mut json = serde_json::to_string_pretty(result)?;
    json.push('\n');
    std::fs::write(&path, json).wrap_err_with(|| format!("couldn't write {}", path.display()))?;
    Ok(path)
}

/// Every result in `dir`, oldest first.
pub fn load_all(dir: &Path) -> Vec<(PathBuf, RunResult)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut all: Vec<(PathBuf, RunResult)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| {
            let result = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            Some((path, result))
        })
        .collect();
    all.sort_by_key(|(_, result)| result.started_ms);
    all
}

pub fn load(path: &Path) -> Result<RunResult> {
    let bytes =
        std::fs::read(path).wrap_err_with(|| format!("couldn't read {}", path.display()))?;
    serde_json::from_slice(&bytes).wrap_err_with(|| format!("{} doesn't read", path.display()))
}

/// One task compared with the run before.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub id: String,
    pub lang: Lang,
    pub before: Option<Status>,
    pub now: Status,
    pub first_before: Option<u64>,
    pub first_now: Option<u64>,
    pub total_before: Option<u64>,
    pub total_now: Option<u64>,
    pub steps_before: Option<usize>,
    pub steps_now: Option<usize>,
}

impl Row {
    /// Passed before, and doesn't now.
    pub fn regressed(&self) -> bool {
        self.before == Some(Status::Passed) && !matches!(self.now, Status::Passed | Status::Skipped)
    }

    pub fn fixed(&self) -> bool {
        self.now == Status::Passed
            && self
                .before
                .is_some_and(|before| !matches!(before, Status::Passed | Status::Skipped))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Comparison {
    pub rows: Vec<Row>,
    pub passed_before: Option<usize>,
    pub passed_now: usize,
    pub ran_now: usize,
    pub median_first_before: Option<u64>,
    pub median_first_now: Option<u64>,
    pub median_total_before: Option<u64>,
    pub median_total_now: Option<u64>,
}

impl Comparison {
    /// Worse than before: a task that passed fails now, or fewer pass.
    pub fn worse(&self) -> bool {
        self.rows.iter().any(Row::regressed)
            || self
                .passed_before
                .is_some_and(|before| self.passed_now < before)
    }
}

pub fn compare(now: &RunResult, before: Option<&RunResult>) -> Comparison {
    let rows = now
        .tasks
        .iter()
        .map(|task| {
            let old =
                before.and_then(|before| before.tasks.iter().find(|old| old.key() == task.key()));
            Row {
                id: task.id.clone(),
                lang: task.lang,
                before: old.map(|old| old.status),
                now: task.status,
                first_before: old.and_then(|old| old.first_answer_ms),
                first_now: task.first_answer_ms,
                total_before: old.and_then(|old| old.total_ms),
                total_now: task.total_ms,
                steps_before: old.and_then(|old| old.steps),
                steps_now: task.steps,
            }
        })
        .collect();
    // Only tasks both runs had count for “fewer pass”, so a filtered run
    // isn't worse than a full one.
    let passed_before = before.map(|before| {
        before
            .tasks
            .iter()
            .filter(|old| old.status == Status::Passed)
            .filter(|old| now.tasks.iter().any(|task| task.key() == old.key()))
            .count()
    });
    Comparison {
        rows,
        passed_before,
        passed_now: now.passed(),
        ran_now: now.ran(),
        median_first_before: before.and_then(|before| before.median(|task| task.first_answer_ms)),
        median_first_now: now.median(|task| task.first_answer_ms),
        median_total_before: before.and_then(|before| before.median(|task| task.total_ms)),
        median_total_now: now.median(|task| task.total_ms),
    }
}

fn ms(value: Option<u64>) -> String {
    value.map_or_else(|| "-".into(), |value| format!("{value}"))
}

fn arrow(before: Option<u64>, now: Option<u64>) -> String {
    match (before, now) {
        (Some(before), Some(now)) if before > 0 => {
            let change = (now as f64 - before as f64) / before as f64 * 100.0;
            if change.abs() < 5.0 {
                String::new()
            } else {
                format!(" ({change:+.0}%)")
            }
        }
        _ => String::new(),
    }
}

/// The comparison as a table for a person.
pub fn table(now: &RunResult, before: Option<&RunResult>, comparison: &Comparison) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Run {} · {} · {} ({})\n",
        now.run_id,
        now.version,
        now.target,
        super::timestamp(now.started_ms)
    ));
    match before {
        Some(before) => out.push_str(&format!(
            "Compared with {} · {} ({})\n\n",
            before.run_id,
            before.version,
            super::timestamp(before.started_ms)
        )),
        None => out.push_str("No earlier run to compare with.\n\n"),
    }
    out.push_str(&format!(
        "{:<28} {:<3} {:>8} {:>8} {:>16} {:>16} {:>9}\n",
        "task", "", "before", "now", "first ms", "total ms", "steps"
    ));
    for row in &comparison.rows {
        let mark = if row.regressed() {
            " ▼"
        } else if row.fixed() {
            " ▲"
        } else {
            ""
        };
        out.push_str(&format!(
            "{:<28} {:<3} {:>8} {:>8} {:>16} {:>16} {:>9}{mark}\n",
            super::shorten(&row.id, 28),
            row.lang.code(),
            row.before.map_or("-", Status::label),
            row.now.label(),
            format!(
                "{}{}",
                ms(row.first_now),
                arrow(row.first_before, row.first_now)
            ),
            format!(
                "{}{}",
                ms(row.total_now),
                arrow(row.total_before, row.total_now)
            ),
            row.steps_now
                .map_or_else(|| "-".into(), |steps| steps.to_string()),
        ));
    }
    out.push_str(&format!(
        "\nPassed {}/{} (before: {}). Median first answer {} ms (before {}), total {} ms (before {}).\n",
        comparison.passed_now,
        comparison.ran_now,
        comparison
            .passed_before
            .map_or_else(|| "-".into(), |passed| passed.to_string()),
        ms(comparison.median_first_now),
        ms(comparison.median_first_before),
        ms(comparison.median_total_now),
        ms(comparison.median_total_before),
    ));
    out.push_str(if comparison.worse() {
        "Worse than before.\n"
    } else {
        "Not worse than before.\n"
    });
    for task in &now.tasks {
        if matches!(
            task.status,
            Status::Failed | Status::Timeout | Status::Error
        ) {
            out.push_str(&format!(
                "  {} {} [{}]: {}\n",
                task.id,
                task.lang.code(),
                task.status.label(),
                task.reason
            ));
        }
    }
    out
}
