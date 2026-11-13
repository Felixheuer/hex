//! A benchmark task as data (`bench/tasks/*.toml`): what the person says in
//! German and English, which lane the assistant should take, how success is
//! checked by machine, and what the run may touch.
//!
//! [`validate`] holds the safety rules every task must keep: messages only
//! as drafts or to the configured test recipients, files only under the
//! run's temp folder, no personal addresses or numbers in the text, and the
//! person's front window untouched unless the run allows otherwise.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use color_eyre::{Result, eyre::WrapErr, eyre::eyre};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// The longest a task may take, in seconds.
pub const MAX_TIMEOUT_S: u64 = 1800;
/// The shortest timeout a task may set, in seconds.
pub const MIN_TIMEOUT_S: u64 = 5;

/// The placeholders the runner fills itself, per task.
pub const RUN_PLACEHOLDERS: &[&str] = &["temp_dir", "run"];

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    De,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
        }
    }
}

/// Which lane the assistant should take for the task.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LaneExpect {
    /// Answered in the quick turn, no job started.
    Quick,
    /// Handed to a background job.
    Job,
    /// Either is fine.
    #[default]
    Any,
}

/// What a task may do with messages.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Messages {
    /// It writes no message at all.
    #[default]
    None,
    /// It drafts a message to a test recipient and never sends it.
    Draft,
}

/// Whether the person's front app may change.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Front {
    /// The front app stays the same through the whole task (checked).
    #[default]
    Keep,
    /// The task brings an app forward (switch to, reveal in Finder): it runs
    /// only with `--allow-front`, when nobody uses the Mac.
    MayChange,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Safety {
    #[serde(default)]
    pub messages: Messages,
    #[serde(default)]
    pub front: Front,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spoken {
    pub de: String,
    pub en: String,
}

impl Spoken {
    pub fn get(&self, lang: Lang) -> &str {
        match lang {
            Lang::De => &self.de,
            Lang::En => &self.en,
        }
    }
}

/// A long job started first, so the task is asked while it runs.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Background {
    pub de: String,
    pub en: String,
    /// How long after the job starts the task is asked.
    #[serde(default = "default_lead_ms")]
    pub lead_ms: u64,
}

fn default_lead_ms() -> u64 {
    4000
}

/// A file the runner puts in the task's temp folder before asking.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    pub name: String,
    #[serde(default)]
    pub content: String,
}

/// Where a `file` check looks.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileRoot {
    /// The task's temp folder under `~/Documents/Agent-Desk/bench`.
    #[default]
    Temp,
    /// The job's own folder (its workspace).
    Workspace,
}

fn yes() -> bool {
    true
}

fn one() -> usize {
    1
}

/// One machine check. Patterns are regular expressions; `{key}`
/// placeholders in them are filled (escaped) like the spoken text.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Check {
    /// The request ended answered (not failed, cancelled, or timed out).
    Answered,
    /// The answer text matches `pattern` (only for `lang` when set).
    AnswerMatches {
        pattern: String,
        #[serde(default)]
        lang: Option<Lang>,
    },
    /// The answer is in the language the task was asked in.
    AnswerLanguage,
    /// The first answer came within `ms` of asking.
    AnswerWithinMs { ms: u64 },
    /// At most `steps` tool calls.
    MaxSteps { steps: usize },
    /// `app` runs afterwards.
    AppRunning { app: String },
    /// The front app afterwards is (or with `is = false`, is not) `app`.
    FrontApp {
        app: String,
        #[serde(default = "yes")]
        is: bool,
    },
    /// A window of `app` (whose title matches `title`, and that is or isn't
    /// visible on this Space) exists, or with `present = false`, doesn't.
    Window {
        app: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default = "yes")]
        present: bool,
    },
    /// At least `min` files whose names match the glob `glob` (with `*` and
    /// `?`) under the temp folder or the job's folder, one of them holding
    /// text that matches `contains`.
    File {
        glob: String,
        #[serde(default)]
        contains: Option<String>,
        #[serde(default = "one")]
        min: usize,
        #[serde(default, rename = "in")]
        root: FileRoot,
    },
    /// At least `min` distinct web addresses the task opened match
    /// `pattern` (from its tool calls and steps).
    BrowserUrl {
        pattern: String,
        #[serde(default = "one")]
        min: usize,
    },
    /// Some tool call (its name and arguments) matches `pattern`.
    ToolUsed { pattern: String },
    /// No tool call matches `pattern` (a send, a key press of Return).
    ToolNotUsed { pattern: String },
    /// The job wrote a plan of at least `min_steps` steps (all done with
    /// `all_done`).
    Plan {
        #[serde(default = "one")]
        min_steps: usize,
        #[serde(default)]
        all_done: bool,
    },
    /// With `[background]`: answered while that job still ran.
    WhileRunning,
    /// With `after`: continued the earlier task's conversation.
    SameConversation,
}

impl Check {
    /// A short label for tables and reports.
    pub fn label(&self) -> String {
        match self {
            Self::Answered => "answered".into(),
            Self::AnswerMatches { pattern, .. } => format!("answer ~ /{pattern}/"),
            Self::AnswerLanguage => "answer language".into(),
            Self::AnswerWithinMs { ms } => format!("first answer ≤ {ms} ms"),
            Self::MaxSteps { steps } => format!("≤ {steps} steps"),
            Self::AppRunning { app } => format!("{app} runs"),
            Self::FrontApp { app, is } => {
                format!("front app {} {app}", if *is { "is" } else { "isn't" })
            }
            Self::Window { app, present, .. } => {
                format!(
                    "{app} window {}",
                    if *present { "exists" } else { "absent" }
                )
            }
            Self::File { glob, min, .. } => format!("≥ {min} file {glob}"),
            Self::BrowserUrl { pattern, min } => format!("≥ {min} url ~ /{pattern}/"),
            Self::ToolUsed { pattern } => format!("tool ~ /{pattern}/"),
            Self::ToolNotUsed { pattern } => format!("no tool ~ /{pattern}/"),
            Self::Plan { min_steps, .. } => format!("plan ≥ {min_steps} steps"),
            Self::WhileRunning => "while the job runs".into(),
            Self::SameConversation => "same conversation".into(),
        }
    }

    /// The patterns it holds, for validation.
    fn patterns(&self) -> Vec<&str> {
        match self {
            Self::AnswerMatches { pattern, .. }
            | Self::BrowserUrl { pattern, .. }
            | Self::ToolUsed { pattern }
            | Self::ToolNotUsed { pattern } => vec![pattern],
            Self::Window {
                title: Some(title), ..
            } => vec![title],
            Self::File {
                contains: Some(contains),
                ..
            } => vec![contains],
            _ => Vec::new(),
        }
    }

    /// Whether it only reads the desktop (allowed as a precondition).
    fn reads_desktop_only(&self) -> bool {
        matches!(
            self,
            Self::AppRunning { .. } | Self::Window { .. } | Self::FrontApp { .. }
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub title: String,
    /// The roadmap item it covers (docs/roadmap-agent-desk.md, G).
    #[serde(default)]
    pub covers: String,
    pub text: Spoken,
    #[serde(default)]
    pub lane: LaneExpect,
    pub timeout_s: u64,
    #[serde(default)]
    pub safety: Safety,
    /// A follow-up: continues this earlier task's conversation.
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub background: Option<Background>,
    #[serde(default)]
    pub seed: Vec<Seed>,
    /// Read-only desktop checks that must hold before asking; when one
    /// doesn't, the task is skipped (the Mac isn't set up for it).
    #[serde(default)]
    pub precondition: Vec<Check>,
    #[serde(rename = "check")]
    pub checks: Vec<Check>,
    /// The file it came from.
    #[serde(skip)]
    pub file: PathBuf,
}

/// The person's bench settings (`bench/config.toml`, not committed): test
/// recipients and what their Mac holds for the reading tasks.
pub type Config = BTreeMap<String, String>;

/// The keys whose values are message recipients.
pub const RECIPIENT_KEYS: &[&str] = &["test_contact", "test_email"];

/// Reads `bench/config.toml`, or `None` when there is none.
pub fn load_config(path: &Path) -> Result<Option<Config>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("couldn't read {}", path.display()));
        }
    };
    parse_config(&text)
        .map(Some)
        .wrap_err_with(|| format!("{} doesn't read", path.display()))
}

pub fn parse_config(text: &str) -> Result<Config> {
    let table: toml::Table = toml::from_str(text)?;
    let mut config = Config::new();
    for (key, value) in table {
        let value = match value {
            toml::Value::String(text) => text,
            toml::Value::Integer(number) => number.to_string(),
            other => return Err(eyre!("`{key}` must be text, not {}", other.type_str())),
        };
        if !value.trim().is_empty() {
            config.insert(key, value);
        }
    }
    Ok(config)
}

/// Every `bench/tasks/*.toml`, in file name order.
pub fn load_dir(dir: &Path) -> Result<Vec<Task>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .wrap_err_with(|| format!("couldn't list {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|file| {
            let text = std::fs::read_to_string(&file)
                .wrap_err_with(|| format!("couldn't read {}", file.display()))?;
            let mut task = parse(&text).wrap_err_with(|| format!("{}", file.display()))?;
            task.file = file;
            Ok(task)
        })
        .collect()
}

pub fn parse(text: &str) -> Result<Task> {
    Ok(toml::from_str(text)?)
}

/// `{key}` placeholders in `text`.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    static PLACEHOLDER: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PLACEHOLDER
        .get_or_init(|| Regex::new(r"\{([a-z_][a-z0-9_]*)\}").expect("valid"))
        .captures_iter(text)
        .map(|captures| captures[1].to_owned())
        .collect()
}

/// `text` with each `{key}` replaced by `values[key]` (escaped for a
/// regular expression with `escape`, except keys ending in `_re`, which
/// are expressions themselves); `Err` names a missing key.
pub fn fill(
    text: &str,
    values: &BTreeMap<String, String>,
    escape: bool,
) -> std::result::Result<String, String> {
    let mut out = text.to_owned();
    for key in placeholders(text) {
        let value = values.get(&key).ok_or_else(|| key.clone())?;
        let value = if escape && !key.ends_with("_re") {
            regex::escape(value)
        } else {
            value.clone()
        };
        out = out.replace(&format!("{{{key}}}"), &value);
    }
    Ok(out)
}

impl Task {
    /// Everything spoken: the text and the background job's, in both
    /// languages.
    pub fn spoken(&self) -> Vec<&str> {
        let mut spoken = vec![self.text.de.as_str(), self.text.en.as_str()];
        if let Some(background) = &self.background {
            spoken.extend([background.de.as_str(), background.en.as_str()]);
        }
        spoken.extend(self.seed.iter().map(|seed| seed.name.as_str()));
        spoken
    }

    /// The config keys it needs (placeholders other than the runner's own).
    pub fn needs(&self) -> BTreeSet<String> {
        let mut texts: Vec<&str> = self.spoken();
        texts.extend(self.seed.iter().map(|seed| seed.content.as_str()));
        let mut patterns = Vec::new();
        for check in self.checks.iter().chain(&self.precondition) {
            patterns.extend(check.patterns());
            match check {
                Check::AppRunning { app }
                | Check::FrontApp { app, .. }
                | Check::Window { app, .. } => {
                    patterns.push(app);
                }
                Check::File { glob, .. } => patterns.push(glob),
                _ => {}
            }
        }
        texts
            .into_iter()
            .chain(patterns)
            .flat_map(placeholders)
            .filter(|key| !RUN_PLACEHOLDERS.contains(&key.as_str()))
            .collect()
    }

    /// The config keys it needs that `config` lacks.
    pub fn missing(&self, config: &Config) -> Vec<String> {
        self.needs()
            .into_iter()
            .filter(|key| !config.contains_key(key))
            .collect()
    }
}

/// One rule a task breaks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Problem {
    pub task: String,
    pub message: String,
}

/// Checks every task against the bench's rules (and `config`'s values, when
/// given). An empty list means all may run.
pub fn validate(tasks: &[Task], config: Option<&Config>) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    let id_pattern = Regex::new(r"^[a-z0-9][a-z0-9-]*$").expect("valid");
    // A phone number or an email address spoken literally: messages must
    // name test recipients only through the config.
    let personal = Regex::new(r"(\+?\d[\d \-/]{7,}\d)|([\w.+-]+@[\w-]+\.[\w.]+)").expect("valid");
    // A path outside the run's temp folder.
    let outside = Regex::new(r"(~/|/Users/|/Volumes/|/private/|/etc/|/System/)").expect("valid");
    for task in tasks {
        let mut problem = |message: String| {
            problems.push(Problem {
                task: task.id.clone(),
                message,
            })
        };
        if !id_pattern.is_match(&task.id) {
            problem(format!(
                "id “{}” must be lowercase letters, digits, and dashes",
                task.id
            ));
        }
        if !seen.insert(task.id.clone()) {
            problem("the id is taken by another task".into());
        }
        if task.title.trim().is_empty() {
            problem("it has no title".into());
        }
        for lang in [Lang::De, Lang::En] {
            if task.text.get(lang).trim().is_empty() {
                problem(format!("text.{} is empty", lang.code()));
            }
        }
        if let Some(background) = &task.background
            && (background.de.trim().is_empty() || background.en.trim().is_empty())
        {
            problem("background needs de and en".into());
        }
        if !(MIN_TIMEOUT_S..=MAX_TIMEOUT_S).contains(&task.timeout_s) {
            problem(format!(
                "timeout_s must be {MIN_TIMEOUT_S}..={MAX_TIMEOUT_S}, not {}",
                task.timeout_s
            ));
        }
        if task.checks.is_empty() {
            problem("it has no [[check]]".into());
        }
        for check in task.checks.iter().chain(&task.precondition) {
            for pattern in check.patterns() {
                // Placeholders become escaped literals; check the rest.
                let sample: BTreeMap<String, String> = placeholders(pattern)
                    .into_iter()
                    .map(|key| (key, "x".to_owned()))
                    .collect();
                let filled = fill(pattern, &sample, true).unwrap_or_default();
                if let Err(error) = Regex::new(&filled) {
                    problem(format!("pattern /{pattern}/ doesn't compile: {error}"));
                }
            }
        }
        for check in &task.precondition {
            if !check.reads_desktop_only() {
                problem(format!(
                    "precondition “{}” must only read the desktop (app_running, window, front_app)",
                    check.label()
                ));
            }
        }
        for text in task.spoken() {
            if personal.is_match(text) {
                problem(format!(
                    "“{text}” names a phone number or address; use {{test_contact}} or {{test_email}}"
                ));
            }
            if outside.is_match(text) {
                problem(format!(
                    "“{text}” names a path outside the run's temp folder; use {{temp_dir}}"
                ));
            }
        }
        for seed in &task.seed {
            if seed.name.contains('/') || seed.name.contains("..") || seed.name.trim().is_empty() {
                problem(format!("seed “{}” must be a plain file name", seed.name));
            }
        }
        let says = |key: &str| {
            [&task.text.de, &task.text.en]
                .iter()
                .all(|text| text.contains(&format!("{{{key}}}")))
        };
        let names_recipient = RECIPIENT_KEYS.iter().any(|key| says(key));
        match task.safety.messages {
            Messages::None if names_recipient => {
                problem("it names a test recipient but says safety.messages = \"none\"".into())
            }
            Messages::Draft => {
                if !names_recipient {
                    problem(
                        "a message task must address {test_contact} or {test_email} in both languages"
                            .into(),
                    );
                }
                if !task
                    .checks
                    .iter()
                    .any(|check| matches!(check, Check::ToolNotUsed { .. }))
                {
                    problem("a draft must have a tool_not_used check against sending".into());
                }
            }
            Messages::None => {}
        }
        let writes_temp = task.checks.iter().any(|check| {
            matches!(
                check,
                Check::File {
                    root: FileRoot::Temp,
                    ..
                }
            )
        });
        if writes_temp && !says("temp_dir") && task.seed.is_empty() {
            problem("a file check in the temp folder needs {temp_dir} in both texts".into());
        }
        let has = |wanted: fn(&Check) -> bool| task.checks.iter().any(wanted);
        if has(|check| matches!(check, Check::WhileRunning)) && task.background.is_none() {
            problem("while_running needs a [background] job".into());
        }
        if has(|check| matches!(check, Check::SameConversation)) && task.after.is_none() {
            problem("same_conversation needs `after`".into());
        }
        if let Some(after) = &task.after
            && !tasks
                .iter()
                .take_while(|earlier| earlier.id != task.id)
                .any(|earlier| &earlier.id == after)
        {
            problem(format!("after = “{after}” must name an earlier task"));
        }
        if let Some(config) = config {
            for key in RECIPIENT_KEYS {
                if says(key)
                    && let Some(value) = config.get(*key)
                    && value.trim().len() < 2
                {
                    problem(format!(
                        "config `{key}` is too short to be a test recipient"
                    ));
                }
            }
        }
    }
    problems
}
