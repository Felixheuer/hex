//! What a run saw of one task ([`Observation`]) and whether each of the
//! task's checks holds on it. Pure: everything the checks read is in the
//! observation, so they are tested without an app.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::task::{Check, FileRoot, LaneExpect, Lang, fill};

/// A window as `desktop_list_windows` lists it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Win {
    pub app: String,
    pub title: String,
    pub visible: bool,
}

/// One tool call from the task's audit log.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolCall {
    pub tool: String,
    pub input: String,
}

/// What the run saw of one task.
#[derive(Clone, Debug, Default)]
pub struct Observation {
    pub lang: Option<Lang>,
    /// `answered`, `failed`, `cancelled`, `interrupted`, `running`, or
    /// `timeout`.
    pub outcome: String,
    /// The assistant's text, every message joined.
    pub answer: String,
    pub first_answer_ms: Option<u64>,
    pub total_ms: u64,
    pub tools: Vec<ToolCall>,
    /// The steps as the conversation shows them ("Opened Safari").
    pub actions: Vec<String>,
    /// Whether a job ran (a workspace, a Background origin, or jobs the
    /// turn started).
    pub job: bool,
    pub workspace: Option<PathBuf>,
    /// `plan.md`'s steps: (done, title).
    pub plan: Vec<(bool, String)>,
    pub temp_dir: PathBuf,
    pub front_before: Option<String>,
    /// Every front app seen while the task ran, and after.
    pub fronts: Vec<String>,
    pub front_after: Option<String>,
    pub apps_after: Option<Vec<String>>,
    /// Windows by app, as read after the task.
    pub windows_after: BTreeMap<String, Vec<Win>>,
    /// The background job still ran when the first answer came.
    pub background_running_at_answer: Option<bool>,
    /// The conversation it continued, and the one it ran in.
    pub continued: Option<String>,
    pub conversation: Option<String>,
}

impl Observation {
    pub fn steps(&self) -> usize {
        self.tools.len().max(self.actions.len())
    }

    pub fn front_changed(&self) -> bool {
        match &self.front_before {
            Some(before) => self
                .fronts
                .iter()
                .chain(self.front_after.as_ref())
                .any(|front| front != before),
            None => false,
        }
    }

    /// Web addresses in its tool calls and steps.
    pub fn urls(&self) -> BTreeSet<String> {
        static URL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let url = URL.get_or_init(|| Regex::new(r#"https?://[^\s"'<>\\)\]}]+"#).expect("valid"));
        self.tools
            .iter()
            .map(|call| call.input.as_str())
            .chain(self.actions.iter().map(String::as_str))
            .flat_map(|text| url.find_iter(text).map(|found| found.as_str().to_owned()))
            .collect()
    }
}

/// One check's verdict.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Verdict {
    pub check: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl Verdict {
    fn new(check: String, passed: bool, detail: impl Into<String>) -> Self {
        Self {
            check,
            passed,
            detail: detail.into(),
        }
    }
}

/// The lane check every task gets from its `lane`.
pub fn lane(expect: LaneExpect, seen: &Observation) -> Option<Verdict> {
    let (wanted, label) = match expect {
        LaneExpect::Any => return None,
        LaneExpect::Quick => (false, "lane quick"),
        LaneExpect::Job => (true, "lane job"),
    };
    Some(Verdict::new(
        label.into(),
        seen.job == wanted,
        if seen.job {
            "ran as a job"
        } else {
            "answered in the quick turn"
        },
    ))
}

/// The front-window check every `front = "keep"` task gets.
pub fn front_kept(seen: &Observation) -> Verdict {
    let changed = seen.front_changed();
    Verdict::new(
        "front app untouched".into(),
        !changed,
        if changed {
            format!(
                "{} → {}",
                seen.front_before.as_deref().unwrap_or("?"),
                seen.fronts
                    .iter()
                    .chain(seen.front_after.as_ref())
                    .find(|front| Some(*front) != seen.front_before.as_ref())
                    .map_or("?", String::as_str)
            )
        } else {
            String::new()
        },
    )
}

/// Whether `check` holds on `seen`, with `values` filling placeholders.
pub fn evaluate(check: &Check, seen: &Observation, values: &BTreeMap<String, String>) -> Verdict {
    let label = check.label();
    let pattern = |pattern: &str| -> Result<Regex, String> {
        let filled =
            fill(pattern, values, true).map_err(|key| format!("no value for {{{key}}}"))?;
        Regex::new(&filled).map_err(|error| error.to_string())
    };
    let text = |text: &str| fill(text, values, false).unwrap_or_else(|_| text.to_owned());
    let result = |passed: bool, detail: String| Verdict::new(label.clone(), passed, detail);
    match check {
        Check::Answered => result(seen.outcome == "answered", seen.outcome.clone()),
        Check::AnswerMatches {
            pattern: wanted,
            lang,
        } => {
            if lang.is_some() && *lang != seen.lang {
                return result(true, "other language".into());
            }
            match pattern(wanted) {
                Ok(regex) => result(
                    regex.is_match(&seen.answer),
                    super::shorten(&seen.answer, 120),
                ),
                Err(error) => result(false, error),
            }
        }
        Check::AnswerLanguage => {
            let found = language(&seen.answer);
            result(
                seen.lang.is_some() && found == seen.lang,
                found.map_or("unclear".into(), |lang| lang.code().into()),
            )
        }
        Check::AnswerWithinMs { ms } => match seen.first_answer_ms {
            Some(first) => result(first <= *ms, format!("{first} ms")),
            None => result(false, "no answer".into()),
        },
        Check::MaxSteps { steps } => {
            result(seen.steps() <= *steps, format!("{} steps", seen.steps()))
        }
        Check::AppRunning { app } => {
            let app = text(app);
            match &seen.apps_after {
                Some(apps) => result(
                    apps.iter().any(|running| same_app(running, &app)),
                    String::new(),
                ),
                None => result(false, "couldn't list the apps".into()),
            }
        }
        Check::FrontApp { app, is } => {
            let app = text(app);
            let front = seen.front_after.clone().unwrap_or_default();
            result(same_app(&front, &app) == *is, front)
        }
        Check::Window {
            app,
            title,
            visible,
            present,
        } => {
            let app = text(app);
            let title = match title.as_deref().map(pattern).transpose() {
                Ok(title) => title,
                Err(error) => return result(false, error),
            };
            let Some(windows) = seen
                .windows_after
                .iter()
                .find(|(name, _)| same_app(name, &app))
                .map(|(_, windows)| windows)
            else {
                return result(!present, "no windows read".into());
            };
            let found = windows.iter().any(|window| {
                title
                    .as_ref()
                    .is_none_or(|title| title.is_match(&window.title))
                    && visible.is_none_or(|visible| visible == window.visible)
            });
            result(found == *present, format!("{} windows", windows.len()))
        }
        Check::File {
            glob,
            contains,
            min,
            root,
        } => {
            let dir = match root {
                FileRoot::Temp => Some(seen.temp_dir.clone()),
                FileRoot::Workspace => seen.workspace.clone(),
            };
            let Some(dir) = dir else {
                return result(false, "no job folder".into());
            };
            let glob = text(glob);
            let files = find_files(&dir, &glob, 4);
            let contains = match contains.as_deref().map(pattern).transpose() {
                Ok(contains) => contains,
                Err(error) => return result(false, error),
            };
            let holds = contains.as_ref().is_none_or(|contains| {
                files.iter().any(|file| {
                    readable_text(file).is_some_and(|content| contains.is_match(&content))
                })
            });
            result(
                files.len() >= *min && holds,
                format!("{} found in {}", files.len(), dir.display()),
            )
        }
        Check::BrowserUrl {
            pattern: wanted,
            min,
        } => match pattern(wanted) {
            Ok(regex) => {
                let urls: Vec<String> = seen
                    .urls()
                    .into_iter()
                    .filter(|url| regex.is_match(url))
                    .collect();
                result(urls.len() >= *min, format!("{} matching", urls.len()))
            }
            Err(error) => result(false, error),
        },
        Check::ToolUsed { pattern: wanted } | Check::ToolNotUsed { pattern: wanted } => {
            let regex = match pattern(wanted) {
                Ok(regex) => regex,
                Err(error) => return result(false, error),
            };
            let hit = seen
                .tools
                .iter()
                .find(|call| regex.is_match(&format!("{} {}", call.tool, call.input)));
            let used = matches!(check, Check::ToolUsed { .. });
            result(
                hit.is_some() == used,
                hit.map(|call| call.tool.clone()).unwrap_or_default(),
            )
        }
        Check::Plan {
            min_steps,
            all_done,
        } => {
            let done = seen.plan.iter().filter(|(done, _)| *done).count();
            result(
                seen.plan.len() >= *min_steps && (!all_done || done == seen.plan.len()),
                format!("{done}/{} done", seen.plan.len()),
            )
        }
        Check::WhileRunning => result(
            seen.background_running_at_answer == Some(true),
            match seen.background_running_at_answer {
                Some(true) => "job ran".into(),
                Some(false) => "job had ended".into(),
                None => "no job".into(),
            },
        ),
        Check::SameConversation => result(
            seen.continued.is_some() && seen.continued == seen.conversation,
            seen.conversation.clone().unwrap_or_default(),
        ),
    }
}

/// App names compare without case and the invisible marks some apps carry.
pub fn same_app(a: &str, b: &str) -> bool {
    let clean = |name: &str| {
        name.chars()
            .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{200b}' | '\u{feff}'))
            .collect::<String>()
            .trim()
            .to_lowercase()
    };
    !b.trim().is_empty() && clean(a) == clean(b)
}

/// German or English, by common words; `None` when it can't tell.
pub fn language(text: &str) -> Option<Lang> {
    const DE: &[&str] = &[
        "der", "die", "das", "und", "ist", "nicht", "ich", "es", "ein", "eine", "zu", "mit", "auf",
        "für", "den", "dem", "sich", "auch", "sind", "wird", "habe", "du", "dir", "ihr", "heute",
        "morgen", "uhr", "kein", "keine", "noch", "schon", "wie", "was", "im",
    ];
    const EN: &[&str] = &[
        "the", "and", "is", "not", "it", "a", "an", "to", "with", "on", "for", "of", "are", "you",
        "your", "i", "have", "has", "this", "that", "be", "will", "today", "tomorrow", "no", "yet",
        "already", "how", "what", "in", "at",
    ];
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let count = |list: &[&str]| {
        words
            .iter()
            .filter(|word| list.contains(&word.as_str()))
            .count()
    };
    let umlauts = text.chars().filter(|c| "äöüßÄÖÜ".contains(*c)).count();
    let (de, en) = (count(DE) + umlauts, count(EN));
    if de + en == 0 {
        return None;
    }
    match de.cmp(&en) {
        std::cmp::Ordering::Greater => Some(Lang::De),
        std::cmp::Ordering::Less => Some(Lang::En),
        std::cmp::Ordering::Equal => None,
    }
}

/// `*` and `?` wildcards, without case.
pub fn glob_match(glob: &str, name: &str) -> bool {
    let glob: Vec<char> = glob.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    let (mut g, mut n) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while n < name.len() {
        if g < glob.len() && (glob[g] == '?' || glob[g] == name[n]) {
            g += 1;
            n += 1;
        } else if g < glob.len() && glob[g] == '*' {
            star = Some(g);
            mark = n;
            g += 1;
        } else if let Some(at) = star {
            g = at + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    glob[g..].iter().all(|c| *c == '*')
}

/// Files under `dir` (to `depth` folders down) whose names match `glob`.
pub fn find_files(dir: &Path, glob: &str, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // A Numbers document is a package (a folder).
        if glob_match(glob, &name) {
            found.push(path);
        } else if depth > 0 && path.is_dir() {
            found.extend(find_files(&path, glob, depth - 1));
        }
    }
    found.sort();
    found
}

/// A file's text, when it is small text (CSV, Markdown, plain).
fn readable_text(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
        return None;
    }
    String::from_utf8(std::fs::read(path).ok()?).ok()
}
