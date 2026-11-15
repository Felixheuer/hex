//! What a task is doing right now, in one short line in the person's
//! language, when its plan doesn't say: the tool it runs ("Searching the
//! code: link contacts", "Reading setup_gaps.rs", "Running a command ·
//! 1:20" once it runs long), else the first sentence of what it said last
//! ("I'm checking the native contacts query…"). The island, a job's card,
//! and the Assistant pane all read it from here.
//!
//! The tool's kind is read from its name, the same for every backend and
//! app ([`ToolKind::of`]); its detail is a description the tool gave, a
//! file's name, a search, or a site, never a raw command.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::assistant::Event;
use crate::desktop::i18n::{tr, tr_fill};

/// After this long a running tool's line says how long it runs.
pub const SHOW_ELAPSED: Duration = Duration::from_secs(10);
/// A detail's most characters.
const DETAIL_CHARS: usize = 40;
/// How much of what it said is kept, for its latest sentence.
const SAID_KEPT: usize = 600;

/// What kind of thing a tool does, from its name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolKind {
    /// Runs a shell command.
    Command,
    /// Searches files or code (grep, glob, find, list).
    Search,
    /// Reads a file.
    Read,
    /// Writes or edits a file.
    Edit,
    /// Searches the web.
    WebSearch,
    /// Loads a web page without a browser.
    Fetch,
    /// Works in a browser.
    Browser,
    /// Works in an app on the desktop.
    Desktop,
    /// Hands part of it to a helper.
    Helper,
    /// Updates its to-do list.
    Plan,
    /// Anything else.
    Other,
}

impl ToolKind {
    /// The kind of tool `name` is, whoever's tool it is (`Bash`,
    /// `exec_command`, `mcp__server__read_file`, `desktop_click`).
    pub fn of(name: &str) -> Self {
        if crate::assistant::provider::desktop::command(name).is_some()
            || name.ends_with(crate::assistant::guard::OPEN_APP_TOOL)
        {
            return Self::Desktop;
        }
        let lower = name.to_ascii_lowercase();
        if lower.contains("browser") {
            return Self::Browser;
        }
        let short = lower.rsplit("__").next().unwrap_or(&lower);
        let has = |words: &[&str]| words.iter().any(|word| short.contains(word));
        if has(&["websearch", "web_search", "search_web"]) {
            Self::WebSearch
        } else if has(&["webfetch", "web_fetch", "fetch", "http"]) {
            Self::Fetch
        } else if has(&["todo", "plan"]) {
            Self::Plan
        } else if has(&["edit", "write", "patch", "file_change", "create_file"]) {
            Self::Edit
        } else if has(&["bash", "shell", "command", "exec", "terminal", "run"]) {
            Self::Command
        } else if has(&["grep", "glob", "search", "find", "list"]) || short == "ls" {
            Self::Search
        } else if has(&["read", "open_file"]) || matches!(short, "view" | "cat") {
            Self::Read
        } else if matches!(short, "task" | "agent") || has(&["subagent", "delegate"]) {
            Self::Helper
        } else {
            Self::Other
        }
    }
}

/// One tool call as its line says it: its kind and, when safe to show, a
/// detail (its own description, a file's name, a search, a site).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolUse {
    pub kind: ToolKind,
    pub detail: Option<String>,
}

impl ToolUse {
    /// The call of `name` with `input`. A command shows only a description
    /// it gave, never itself (it may hold a secret).
    pub fn of(name: &str, input: &Value) -> Self {
        let kind = ToolKind::of(name);
        let description = field(input, &["description"]);
        let file = || field(input, &["file_path", "filePath", "path", "notebook_path"]);
        let detail = match kind {
            ToolKind::Read | ToolKind::Edit => {
                file().map(base_name).or(description.map(str::to_owned))
            }
            ToolKind::Search | ToolKind::WebSearch => description
                .or_else(|| field(input, &["pattern", "query", "regex"]))
                .map(str::to_owned),
            ToolKind::Fetch | ToolKind::Browser => field(input, &["url"])
                .map(host)
                .or(description.map(str::to_owned)),
            ToolKind::Command | ToolKind::Helper => description.map(str::to_owned),
            ToolKind::Desktop | ToolKind::Plan | ToolKind::Other => None,
        };
        let detail = detail
            .map(|detail| clip(&super::console::redact(&detail), DETAIL_CHARS))
            .filter(|detail| !detail.is_empty());
        Self { kind, detail }
    }

    /// Its line, in the person's language.
    pub fn line(&self) -> String {
        let detail = self.detail.as_deref();
        match (self.kind, detail) {
            (ToolKind::Command, Some(d)) => tr_fill("Running a command: {}", d),
            (ToolKind::Command, None) => tr("Running a command").to_owned(),
            (ToolKind::Search, Some(d)) => tr_fill("Searching the code: {}", d),
            (ToolKind::Search, None) => tr("Searching the code").to_owned(),
            (ToolKind::Read, Some(d)) => tr_fill("Reading {}", d),
            (ToolKind::Read, None) => tr("Reading a file").to_owned(),
            (ToolKind::Edit, Some(d)) => tr_fill("Editing {}", d),
            (ToolKind::Edit, None) => tr("Editing a file").to_owned(),
            (ToolKind::WebSearch, Some(d)) => tr_fill("Searching the web: {}", d),
            (ToolKind::WebSearch, None) => tr("Searching the web").to_owned(),
            (ToolKind::Fetch, Some(d)) => tr_fill("Loading {}", d),
            (ToolKind::Fetch, None) => tr("Loading a page").to_owned(),
            (ToolKind::Browser, Some(d)) => tr_fill("Opening the browser: {}", d),
            (ToolKind::Browser, None) => tr("Working in the browser").to_owned(),
            (ToolKind::Helper, Some(d)) => tr_fill("Helper: {}", d),
            (ToolKind::Helper, None) => tr("Working with a helper").to_owned(),
            (ToolKind::Desktop, _) => tr("Working in an app").to_owned(),
            (ToolKind::Plan, _) => tr("Updating its plan").to_owned(),
            (ToolKind::Other, _) => tr("Using a tool").to_owned(),
        }
    }
}

/// What a task is doing now, between its plan's words: the tool it runs
/// and since when, and what it said last.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Busy {
    tool: Option<(ToolUse, Instant)>,
    /// What it said since its last tool call (or before it), the latest
    /// [`SAID_KEPT`] characters.
    said: String,
    /// A tool ran since it last said something: what it says next starts
    /// afresh.
    fresh: bool,
}

impl Busy {
    /// Takes in `event` (one of its task's); whether its line changed.
    pub fn apply(&mut self, event: &Event, now: Instant) -> bool {
        let before = self.clone();
        match event {
            Event::Started { .. } => *self = Self::default(),
            Event::Tool { tool, .. } => {
                self.tool = Some((tool.clone(), now));
                self.fresh = true;
            }
            Event::ToolDone { .. } | Event::Thinking { .. } => self.tool = None,
            // A step says what it does in its own words, newer than both.
            Event::Step { .. } => {
                self.tool = None;
                self.said.clear();
            }
            Event::AnswerDelta { text, .. } => {
                self.tool = None;
                if std::mem::take(&mut self.fresh) {
                    self.said.clear();
                }
                self.said.push_str(text);
                let extra = self.said.chars().count().saturating_sub(SAID_KEPT);
                if extra > 0 {
                    self.said = self.said.chars().skip(extra).collect();
                }
            }
            _ => return false,
        }
        *self != before
    }

    /// Its line at `now`: the tool it runs (with how long, past
    /// [`SHOW_ELAPSED`]), else the first sentence of what it said last.
    pub fn line(&self, now: Instant) -> Option<String> {
        if let Some((tool, since)) = &self.tool {
            let words = tool.line();
            let ran = now.saturating_duration_since(*since);
            return Some(if ran >= SHOW_ELAPSED {
                format!("{words} · {}", elapsed(ran))
            } else {
                words
            });
        }
        latest_sentence(&self.said, self.fresh)
    }

    /// The tool it runs now, and since when.
    pub fn tool(&self) -> Option<(&ToolUse, Instant)> {
        self.tool.as_ref().map(|(tool, since)| (tool, *since))
    }

    /// When its line next changes by itself (a running tool's time).
    pub fn next_tick(&self, now: Instant) -> Option<Instant> {
        let (_, since) = self.tool.as_ref()?;
        let ran = now.saturating_duration_since(*since);
        Some(if ran < SHOW_ELAPSED {
            *since + SHOW_ELAPSED
        } else {
            *since + Duration::from_secs(ran.as_secs() + 1)
        })
    }
}

/// `ran` as `m:ss` (`h:mm:ss` past an hour).
pub fn elapsed(ran: Duration) -> String {
    let s = ran.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The first sentence of the last paragraph of `said`, plain and short,
/// once it is whole: it ended (`.`, `!`, `?`, `:`, `…`), its line did, or
/// it is `closed` (a tool ran after it). Words still streaming in don't
/// change the line at each one.
fn latest_sentence(said: &str, closed: bool) -> Option<String> {
    let closed = closed || said.ends_with('\n');
    let paragraph = said
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .last()?;
    let mut lines = paragraph.lines().map(super::plain);
    let line = lines.find(|line| !line.is_empty())?;
    let closed = closed || lines.next().is_some();
    let end = line.char_indices().find(|(at, c)| {
        matches!(c, '.' | '!' | '?' | '。' | ':' | '…')
            && line[at + c.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
    });
    if end.is_none() && !closed {
        return None;
    }
    let end = end.map_or(line.len(), |(at, c)| at + c.len_utf8());
    let sentence = line[..end].trim().trim_end_matches(':').trim_end();
    (!sentence.is_empty()).then(|| clip(sentence, super::LINE_CHARS))
}

/// The first of `keys` that `value` has as non-empty text.
pub(super) fn field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    text_of(value, keys).filter(|text| !text.trim().is_empty())
}

/// The first of `keys` that `value` has as text.
pub(super) fn text_of<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
}

/// A path's last part (`setup_gaps.rs` for `src/x/setup_gaps.rs`).
pub(super) fn base_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_owned()
}

/// A URL's host, else the URL.
fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_start_matches("www.").to_owned())
        })
        .unwrap_or_else(|| url.to_owned())
}

/// `text` on one line, at most `max` characters with "…".
fn clip(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        return text;
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}
