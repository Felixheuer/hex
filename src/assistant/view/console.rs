//! A task's terminal: what it ran, searched, read, and edited, as the lines
//! a terminal would show (`$ cargo test`, its output's tail, `✎ edited
//! src/x.rs (+12 −3)` and a few lines of the change). Built from the tool
//! calls every backend reports (`provider::Report::Tool`), for any task:
//! a job's card shows its last lines in a Terminal mini, and a click opens
//! them all. Universal: by the tool's kind ([`ToolKind`]), never by app.
//!
//! Secrets are blanked ([`redact`]) before a line is kept; only commands,
//! their output, searches, reads, and edits make lines, never a message's
//! body. A task keeps at most [`CONSOLE_MAX`] lines.

use std::collections::VecDeque;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::busy::{ToolKind, base_name, field, text_of};

/// The most lines a task's terminal keeps; older ones go.
pub const CONSOLE_MAX: usize = 2000;
/// How many of a command's last output lines it keeps.
pub const OUTPUT_TAIL: usize = 4;
/// How many changed lines an edit shows.
pub const DIFF_LINES: usize = 3;
/// A line's most characters.
const LINE_MAX: usize = 160;

/// What a terminal line is, for its mark and its ink.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Ink {
    /// `$ <command>` (or its description when it gave no command).
    Prompt,
    /// A command's output (dimmed).
    Output,
    /// `✎ edited <file>`, with lines added and removed when known.
    Edited {
        added: Option<usize>,
        removed: Option<usize>,
    },
    /// A line an edit added (green).
    Added,
    /// A line an edit removed (red).
    Removed,
    /// `⌕ searching "<what>"`.
    Searched,
    /// `▤ reading <file>`.
    Read,
}

/// One terminal line: its kind and its words (a command, a file, a query,
/// an output line), secrets blanked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TermLine {
    pub ink: Ink,
    pub text: String,
}

impl TermLine {
    pub fn new(ink: Ink, text: &str) -> Self {
        Self {
            ink,
            text: cut(&redact(text.trim_end()), LINE_MAX),
        }
    }
}

/// A task's terminal lines, oldest first, at most [`CONSOLE_MAX`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Console {
    lines: VecDeque<TermLine>,
}

impl Console {
    /// Adds `lines` at the end; the oldest go past [`CONSOLE_MAX`].
    pub fn push(&mut self, lines: &[TermLine]) {
        self.lines.extend(lines.iter().cloned());
        let extra = self.lines.len().saturating_sub(CONSOLE_MAX);
        self.lines.drain(..extra);
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Every line, oldest first.
    pub fn lines(&self) -> impl Iterator<Item = &TermLine> {
        self.lines.iter()
    }

    /// The last `n` lines, oldest first.
    pub fn tail(&self, n: usize) -> Vec<&TermLine> {
        let skip = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(skip).collect()
    }
}

/// The lines a call of a `kind` tool with `input` makes: its command, its
/// search, its read, or its edit and a few changed lines; none for any
/// other kind (a message typed, a click).
pub fn call_lines(kind: ToolKind, input: &Value) -> Vec<TermLine> {
    match kind {
        ToolKind::Command => command_of(input)
            .or_else(|| field(input, &["description"]).map(str::to_owned))
            .map(|command| vec![TermLine::new(Ink::Prompt, &command)])
            .unwrap_or_default(),
        ToolKind::Search => field(input, &["pattern", "query", "regex"])
            .map(|what| vec![TermLine::new(Ink::Searched, what)])
            .unwrap_or_default(),
        ToolKind::Read => field(input, &["file_path", "filePath", "path", "notebook_path"])
            .map(|file| vec![TermLine::new(Ink::Read, &base_name(file))])
            .unwrap_or_default(),
        ToolKind::Edit => edit_lines(input),
        _ => Vec::new(),
    }
}

/// The last [`OUTPUT_TAIL`] lines of a `kind` tool's `output`: a command's
/// only (what a read or a search returned is the file's, not a terminal's).
pub fn output_lines(kind: ToolKind, output: &str) -> Vec<TermLine> {
    if kind != ToolKind::Command {
        return Vec::new();
    }
    let lines: Vec<String> = output
        .lines()
        .map(strip_ansi)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let skip = lines.len().saturating_sub(OUTPUT_TAIL);
    lines[skip..]
        .iter()
        .map(|line| TermLine::new(Ink::Output, line))
        .collect()
}

/// A command as one line: a string, or an argument list joined.
fn command_of(input: &Value) -> Option<String> {
    let command = match input {
        Value::String(command) => Some(command.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => match input.get("command").or_else(|| input.get("cmd")) {
            Some(Value::String(command)) => Some(command.clone()),
            Some(parts @ Value::Array(_)) => command_of(parts),
            _ => None,
        },
    }?;
    // Its first line: a heredoc's body is not the command.
    let first = command.lines().find(|line| !line.trim().is_empty())?.trim();
    Some(first.to_owned())
}

/// `✎ edited <file> (+a −r)` and up to [`DIFF_LINES`] changed lines: the
/// removed first, then the added.
fn edit_lines(input: &Value) -> Vec<TermLine> {
    // Codex's file changes: a list of paths.
    if let Some(changes) = input.as_array() {
        return changes
            .iter()
            .filter_map(|change| field(change, &["path", "file_path"]))
            .map(|path| TermLine::new(edited(None, None), &base_name(path)))
            .collect();
    }
    let Some(file) = field(input, &["file_path", "filePath", "path", "notebook_path"]) else {
        return Vec::new();
    };
    let mut old = Vec::new();
    let mut new = Vec::new();
    let edits = input.get("edits").and_then(Value::as_array);
    for edit in edits.into_iter().flatten().chain([input]) {
        old.extend(lines_of(edit, &["old_string", "oldString"]));
        new.extend(lines_of(
            edit,
            &["new_string", "newString", "content", "new_source"],
        ));
    }
    let mut lines = vec![TermLine::new(
        edited(Some(new.len()), Some(old.len())),
        &base_name(file),
    )];
    let removed = old.len().min(DIFF_LINES.saturating_sub(new.len().min(2)));
    lines.extend(
        old.iter()
            .take(removed)
            .map(|l| TermLine::new(Ink::Removed, l)),
    );
    let added = DIFF_LINES - removed;
    lines.extend(new.iter().take(added).map(|l| TermLine::new(Ink::Added, l)));
    lines
}

fn edited(added: Option<usize>, removed: Option<usize>) -> Ink {
    Ink::Edited { added, removed }
}

/// The non-empty lines of the first of `keys` that `value` has.
fn lines_of(value: &Value, keys: &[&str]) -> Vec<String> {
    text_of(value, keys)
        .map(|text| {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.trim().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `text` without terminal colour codes.
fn strip_ansi(text: &str) -> String {
    static ANSI: OnceLock<Regex> = OnceLock::new();
    let ansi = ANSI.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("ansi"));
    ansi.replace_all(text, "").into_owned()
}

/// `text` cut to `max` characters with "…".
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

/// What blanks a secret: the pattern, and what stays of it.
fn rules() -> &'static [(Regex, &'static str)] {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            // A password in a URL: `https://me:hunter2@host`.
            (r"(://[^/\s:@]+:)[^/\s@]+@", "${1}•••@"),
            // `Authorization: Bearer …`, `token …`.
            (r"(?i)\b(bearer|basic|token)(\s+)[A-Za-z0-9._~+/=-]{6,}", "${1}${2}•••"),
            // Well-known key shapes, wherever they stand.
            (
                r"\b(sk-[A-Za-z0-9_-]{6,}|sk_(live|test)_[A-Za-z0-9]{6,}|gh[pousr]_[A-Za-z0-9]{6,}|github_pat_[A-Za-z0-9_]{6,}|xox[abprs]-[A-Za-z0-9-]{6,}|AKIA[0-9A-Z]{12,}|AIza[0-9A-Za-z_-]{12,})",
                "•••",
            ),
            // `--password hunter2`, `--token=…`.
            (
                r"(?i)(--?[a-z-]*(?:password|passwd|token|secret|api-?key|key)[ =])\S+",
                "${1}•••",
            ),
            // `API_KEY=…`, `"token": "…"`, `password: …`.
            (
                r#"(?i)([A-Za-z0-9_-]*(?:key|token|secret|password|passwd|pwd|auth|cookie|credential)[A-Za-z0-9_-]*"?)(\s*[=:]\s*)("[^"]*"|'[^']*'|[^\s&"',;]+)"#,
                "${1}${2}•••",
            ),
        ]
        .into_iter()
        .map(|(pattern, keep)| (Regex::new(pattern).expect("redaction rule"), keep))
        .collect()
    })
}

/// `text` with obvious secrets blanked: tokens and keys (`sk-…`, `ghp_…`,
/// `Bearer …`), `key=`/`token=`/`password=` values, and passwords in URLs.
pub fn redact(text: &str) -> String {
    rules().iter().fold(text.to_owned(), |text, (rule, keep)| {
        rule.replace_all(&text, *keep).into_owned()
    })
}

#[cfg(test)]
#[path = "console_tests.rs"]
mod console_tests;
