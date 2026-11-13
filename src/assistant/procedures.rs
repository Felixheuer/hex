//! Procedural memory: what worked in which app, and what didn't, kept per
//! app (and the app's version) and told to the next request that touches
//! that app, so a task starts from the way that worked last time instead of
//! finding it again.
//!
//! Written by itself, never by the agent's say-so alone:
//! - the guard notes the outcome of each action it forwarded ([`Outcome`]):
//!   a menu path, an app action (a URL or a script), a recorded skill
//!   replayed, an Accessibility action on a named control, keys, or a click
//!   by position, and whether the app took it (`guard::proxy::call`);
//! - a job's journal adds what the agent wrote down at the end
//!   ([`from_journal`]: `- worked in <App>: <method> · <how>` and
//!   `- failed in <App>: <method> · <why>`).
//!
//! Read back as a short note in the request's message ([`note`], beside
//! what the memory graph recalls), never in the system prompt: "What worked
//! before in Numbers 14.2: menu File > Export To > PDF (4×) …".
//!
//! Nothing secret is kept: only control names, menu paths, web addresses
//! without their query, and the kind of script, each checked with
//! [`memory::looks_secret`](crate::assistant::memory::looks_secret); never
//! typed text or values.
//!
//! Kept in `<assistant>/procedures/<app>.json`, private to this user.
//!
//! OWNER: brain.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The longest detail kept (a menu path, a control, an address).
const DETAIL_KEPT: usize = 140;
/// The longest reason kept for a failure.
const WHY_KEPT: usize = 120;
/// Entries kept per app; the least useful go first.
const MAX_ENTRIES: usize = 60;
/// Apps one note tells about at most.
const MAX_APPS: usize = 3;
/// What worked, and what didn't, listed per app at most.
const MAX_WORKED: usize = 5;
const MAX_FAILED: usize = 3;

/// How something was done in an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// AppleScript or JavaScript for Automation, or a shell script.
    Script,
    /// A URL (a web address, or the app's own scheme).
    Url,
    /// A menu item by its path.
    Menu,
    /// A recorded skill replayed (`hex_skill_replay`).
    Skill,
    /// An Accessibility action on a named control.
    Ax,
    /// A key combination.
    Keys,
    /// A click or keys at a spot of the window's picture.
    Visual,
    Other,
}

impl Method {
    pub fn word(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Url => "url",
            Self::Menu => "menu",
            Self::Skill => "skill",
            Self::Ax => "accessibility",
            Self::Keys => "keys",
            Self::Visual => "visual",
            Self::Other => "other",
        }
    }

    /// The method a word names ("AppleScript", "AX", "menu", …).
    pub fn parse(word: &str) -> Self {
        let word = word.trim().to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| word.contains(w));
        if has(&["skill", "replay"]) {
            Self::Skill
        } else if has(&["menu"]) {
            Self::Menu
        } else if has(&["script", "jxa", "osascript", "shell", "bash"]) {
            Self::Script
        } else if has(&["url", "link", "scheme", "http"]) {
            Self::Url
        } else if has(&["accessib", "ax", "ref", "button", "click"])
            && !has(&["visual", "pixel", "position", "screenshot"])
        {
            Self::Ax
        } else if has(&["key", "shortcut", "hotkey", "tast"]) {
            Self::Keys
        } else if has(&[
            "visual",
            "pixel",
            "position",
            "screenshot",
            "picture",
            "ocr",
        ]) {
            Self::Visual
        } else {
            Self::Other
        }
    }
}

/// One thing tried in an app, and how it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub app: String,
    pub version: Option<String>,
    pub method: Method,
    /// What exactly: a menu path, "click button “Export”", an address.
    pub detail: String,
    pub ok: bool,
    /// Why it failed.
    pub why: Option<String>,
    /// The project it was learned in, if the task belongs to one.
    pub project: Option<String>,
}

/// One remembered way of doing something in an app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub method: Method,
    pub detail: String,
    /// The app's version the last time it was done.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub worked: u32,
    #[serde(default)]
    pub failed: u32,
    /// The last failure's reason.
    #[serde(default)]
    pub why: Option<String>,
    /// Whether the last time worked.
    #[serde(default)]
    pub last_ok: bool,
    #[serde(default)]
    pub last_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// What is remembered about one app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppMemory {
    pub app: String,
    /// The newest version seen.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

/// `<assistant>/procedures`; `None` in tests, which never write the
/// person's memory.
pub fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let dir = crate::assistant::store::assistant_dir()
        .ok()?
        .join("procedures");
    crate::app_paths::create_private_dir(&dir).ok()?;
    Some(dir)
}

/// An app's name as compared and as a file name: lowercase letters and
/// digits, words joined by `-`, without invisible marks.
pub fn app_slug(app: &str) -> String {
    let mut slug = String::new();
    for c in app.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty()
            && !slug.ends_with('-')
            && (c.is_whitespace() || c.is_ascii_punctuation())
        {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').chars().take(60).collect()
}

/// `text` safe to keep: one line, cut, a web address without its query or
/// fragment (where tokens hide); `None` when it looks like a secret.
pub fn safe_detail(text: &str, max: usize) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = line
        .split(' ')
        .map(|word| {
            if word.contains("://") {
                word.split(['?', '#']).next().unwrap_or(word).to_owned()
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if line.is_empty() || crate::assistant::memory::looks_secret(&line) {
        return None;
    }
    Some(crate::assistant::store::shorten(&line, max))
}

fn path_in(dir: &Path, app: &str) -> Option<PathBuf> {
    let slug = app_slug(app);
    (!slug.is_empty()).then(|| dir.join(format!("{slug}.json")))
}

/// What is remembered about `app` in `dir`.
pub fn load(dir: &Path, app: &str) -> Option<AppMemory> {
    let path = path_in(dir, app)?;
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn save(dir: &Path, memory: &AppMemory) {
    let Some(path) = path_in(dir, &memory.app) else {
        return;
    };
    let Ok(bytes) = serde_json::to_vec_pretty(memory) else {
        return;
    };
    // Written whole and renamed: guards of several tasks write at once.
    let partial = path.with_extension(format!("json.{}.partial", std::process::id()));
    if crate::assistant::store::write_private(&partial, &bytes).is_ok() {
        let _ = std::fs::rename(&partial, &path);
    } else {
        let _ = std::fs::remove_file(&partial);
    }
}

/// Notes `outcome` in `dir` at `now_ms`; whether it was kept (nothing
/// that looks secret is).
pub fn record_in(dir: &Path, outcome: &Outcome, now_ms: u64) -> bool {
    let app = outcome
        .app
        .trim()
        .replace(['\u{200e}', '\u{200f}', '\u{200b}'], "");
    if app_slug(&app).is_empty() {
        return false;
    }
    let Some(detail) = safe_detail(&outcome.detail, DETAIL_KEPT) else {
        return false;
    };
    let why = outcome
        .why
        .as_deref()
        .and_then(|why| safe_detail(why, WHY_KEPT));
    let version = outcome
        .version
        .as_deref()
        .map(str::trim)
        .filter(|version| !version.is_empty() && !crate::assistant::memory::looks_secret(version))
        .map(str::to_owned);
    let mut memory = load(dir, &app).unwrap_or_else(|| AppMemory {
        app: app.clone(),
        ..AppMemory::default()
    });
    if version.is_some() {
        memory.version.clone_from(&version);
    }
    let found = memory.entries.iter().position(|entry| {
        entry.method == outcome.method && entry.detail.eq_ignore_ascii_case(&detail)
    });
    let index = match found {
        Some(index) => index,
        None => {
            memory.entries.push(Entry {
                method: outcome.method,
                detail,
                version: None,
                worked: 0,
                failed: 0,
                why: None,
                last_ok: false,
                last_ms: 0,
                project: None,
            });
            memory.entries.len() - 1
        }
    };
    let entry = &mut memory.entries[index];
    if outcome.ok {
        entry.worked += 1;
    } else {
        entry.failed += 1;
        entry.why = why.or(entry.why.take());
    }
    entry.last_ok = outcome.ok;
    entry.last_ms = now_ms;
    if version.is_some() {
        entry.version = version;
    }
    if outcome.project.is_some() {
        entry.project.clone_from(&outcome.project);
    }
    if memory.entries.len() > MAX_ENTRIES {
        // The least useful go: failed most, worked least, oldest.
        memory
            .entries
            .sort_by_key(|entry| std::cmp::Reverse((usefulness(entry), entry.last_ms)));
        memory.entries.truncate(MAX_ENTRIES);
    }
    save(dir, &memory);
    true
}

/// Notes `outcome` in the person's procedural memory.
pub fn record(outcome: &Outcome) -> bool {
    dir().is_some_and(|dir| record_in(&dir, outcome, crate::assistant::store::now_ms()))
}

fn usefulness(entry: &Entry) -> i64 {
    i64::from(entry.worked) * 2 + i64::from(entry.failed.min(3))
}

/// The apps remembered in `dir`.
fn apps(dir: &Path) -> Vec<AppMemory> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok())
        .collect()
}

/// Whether `text` names `app`: its whole name, or its last word when that
/// is distinctive ("Excel" names "Microsoft Excel"), as words.
pub fn names_app(text: &str, app: &str) -> bool {
    let words = format!("-{}-", app_slug(text));
    let slug = app_slug(app);
    if slug.is_empty() {
        return false;
    }
    if words.contains(&format!("-{slug}-")) {
        return true;
    }
    slug.rsplit('-')
        .next()
        .filter(|last| last.chars().count() >= 4 && *last != slug)
        .is_some_and(|last| words.contains(&format!("-{last}-")))
}

/// The note for a request whose words are `text` (and that works in
/// `apps`, such as the window in front): what worked and failed before in
/// each app it names, from `dir`.
pub fn note_in(dir: &Path, text: &str, hints: &[&str]) -> Option<String> {
    let mut known = apps(dir);
    known.sort_by(|a, b| a.app.cmp(&b.app));
    let touched: Vec<&AppMemory> = known
        .iter()
        .filter(|memory| {
            names_app(text, &memory.app)
                || hints
                    .iter()
                    .any(|hint| app_slug(hint) == app_slug(&memory.app))
        })
        .filter(|memory| !memory.entries.is_empty())
        .take(MAX_APPS)
        .collect();
    let notes: Vec<String> = touched.into_iter().filter_map(app_note).collect();
    (!notes.is_empty()).then(|| notes.join("\n"))
}

/// The note for a request (from the person's procedural memory).
pub fn note(text: &str, hints: &[&str]) -> Option<String> {
    note_in(&dir()?, text, hints)
}

fn app_note(memory: &AppMemory) -> Option<String> {
    let current = memory.version.as_deref();
    let in_version = |entry: &Entry| match (&entry.version, current) {
        (Some(version), Some(current)) if version != current => format!(" (in {version})"),
        _ => String::new(),
    };
    let mut worked: Vec<&Entry> = memory
        .entries
        .iter()
        .filter(|entry| entry.worked > 0 && entry.last_ok)
        .collect();
    // This version's first, then what worked most, then the newest.
    worked.sort_by_key(|entry| {
        (
            std::cmp::Reverse(entry.version.as_deref() == current),
            std::cmp::Reverse(entry.worked),
            std::cmp::Reverse(entry.last_ms),
        )
    });
    let mut failed: Vec<&Entry> = memory
        .entries
        .iter()
        .filter(|entry| entry.failed > 0 && !entry.last_ok)
        .collect();
    failed.sort_by_key(|entry| {
        (
            std::cmp::Reverse(entry.failed),
            std::cmp::Reverse(entry.last_ms),
        )
    });
    if worked.is_empty() && failed.is_empty() {
        return None;
    }
    let name = match current {
        Some(version) => format!("{} {version}", memory.app),
        None => memory.app.clone(),
    };
    let mut out = format!("What worked before in {name}:");
    for entry in worked.iter().take(MAX_WORKED) {
        let skill = if entry.method == Method::Skill {
            " (hex_skill_replay)"
        } else {
            ""
        };
        out.push_str(&format!(
            "\n- {} {}{skill} — worked {}×{}",
            entry.method.word(),
            entry.detail,
            entry.worked,
            in_version(entry)
        ));
    }
    if worked.is_empty() {
        out.push_str("\n- (nothing recorded as working yet)");
    }
    if !failed.is_empty() {
        out.push_str("\nFailed there, so try another way first:");
        for entry in failed.iter().take(MAX_FAILED) {
            let why = entry
                .why
                .as_deref()
                .map(|why| format!(": {why}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "\n- {} {} — failed {}×{why}{}",
                entry.method.word(),
                entry.detail,
                entry.failed,
                in_version(entry)
            ));
        }
    }
    Some(out)
}

/// What a job's journal says worked or failed in an app: the agent's
/// `- worked in <App>: <method> · <how>` and `- failed in <App>: <method>
/// · <why>` lines.
pub fn from_journal(text: &str, project: Option<&str>) -> Vec<Outcome> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start().strip_prefix(['-', '*'])?.trim();
            let (ok, rest) = if let Some(rest) = line.strip_prefix("worked in ") {
                (true, rest)
            } else {
                (false, line.strip_prefix("failed in ")?)
            };
            let (app, rest) = rest.split_once(':')?;
            let mut parts = rest.split(" · ").map(str::trim);
            let method = parts.next().filter(|method| !method.is_empty())?;
            let how: Vec<&str> = parts.collect();
            let (detail, why) = if ok {
                (how.join(" · "), None)
            } else {
                let mut how = how;
                let why = (how.len() > 1).then(|| how.pop().unwrap_or_default().to_owned());
                (how.join(" · "), why)
            };
            Some(Outcome {
                app: app.trim().to_owned(),
                version: None,
                method: Method::parse(method),
                detail: if detail.is_empty() {
                    method.to_owned()
                } else {
                    detail
                },
                ok,
                why,
                project: project.map(str::to_owned),
            })
        })
        .collect()
}

/// Keeps what `journal` (a job's journal's text) says worked or failed;
/// how many were kept.
pub fn learn_from_journal(journal: &str, project: Option<&str>) -> usize {
    let Some(dir) = dir() else {
        return 0;
    };
    let now = crate::assistant::store::now_ms();
    from_journal(journal, project)
        .iter()
        .filter(|outcome| record_in(&dir, outcome, now))
        .count()
}

/// The version of the app bundle at `path` (`CFBundleShortVersionString`).
pub fn bundle_version(path: &Path) -> Option<String> {
    if cfg!(test) {
        return None;
    }
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(path.join("Contents/Info.plist"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!version.is_empty() && version.len() < 40).then_some(version)
}

/// The rules line telling a job how to note what worked in an app.
pub const JOURNAL_RULE: &str = " A way that worked or failed in an app: `- worked in <App>: <method> · <how>` or `- failed in <App>: <method> · <how> · <why>` (method: script, url, menu, skill, accessibility, keys, visual; never typed text); the next task there is told.";

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hex-procedures-{name}-{}-{}",
            std::process::id(),
            crate::assistant::store::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn outcome(app: &str, method: Method, detail: &str, ok: bool) -> Outcome {
        Outcome {
            app: app.into(),
            version: Some("14.2".into()),
            method,
            detail: detail.into(),
            ok,
            why: (!ok).then(|| "element not found".into()),
            project: None,
        }
    }

    #[test]
    fn what_worked_is_written_per_app_and_told_to_the_next_request() {
        let dir = folder("write-read");
        for _ in 0..3 {
            assert!(record_in(
                &dir,
                &outcome("Numbers", Method::Menu, "File > Export To > PDF…", true),
                10
            ));
        }
        assert!(record_in(
            &dir,
            &outcome("Numbers", Method::Visual, "click by position", false),
            20
        ));
        assert!(record_in(
            &dir,
            &outcome("Numbers", Method::Visual, "click by position", false),
            30
        ));
        assert!(record_in(
            &dir,
            &outcome("Numbers", Method::Skill, "export pdf", true),
            40
        ));
        assert!(record_in(
            &dir,
            &outcome("\u{200e}WhatsApp", Method::Url, "whatsapp://send", true),
            50
        ));
        let memory = load(&dir, "numbers").unwrap();
        assert_eq!(memory.version.as_deref(), Some("14.2"));
        assert_eq!(memory.entries.len(), 3, "one entry per method and detail");
        let note = note_in(&dir, "Exportier die Tabelle in Numbers als PDF", &[]).unwrap();
        assert!(
            note.starts_with("What worked before in Numbers 14.2:"),
            "{note}"
        );
        assert!(
            note.contains("- menu File > Export To > PDF… — worked 3×"),
            "{note}"
        );
        assert!(
            note.contains("skill export pdf (hex_skill_replay)"),
            "{note}"
        );
        assert!(
            note.contains("- visual click by position — failed 2×: element not found"),
            "{note}"
        );
        // The most used first.
        assert!(note.find("menu File").unwrap() < note.find("skill export").unwrap());
        // An app not named isn't told; a hint (the window in front) is.
        assert!(!note.contains("WhatsApp"));
        assert_eq!(note_in(&dir, "Wie wird das Wetter?", &[]), None);
        assert!(
            note_in(&dir, "schick das", &["WhatsApp"])
                .unwrap()
                .contains("whatsapp://send")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_version_s_way_is_marked_and_a_later_failure_replaces_success() {
        let dir = folder("version");
        let mut old = outcome("Microsoft Excel", Method::Menu, "Data > Import", true);
        old.version = Some("16.7".into());
        record_in(&dir, &old, 1);
        record_in(
            &dir,
            &outcome("Microsoft Excel", Method::Ax, "click AXButton “Save”", true),
            2,
        );
        // "Excel" names "Microsoft Excel".
        let note = note_in(&dir, "Open the Excel file", &[]).unwrap();
        assert!(note.contains("Microsoft Excel 14.2"), "{note}");
        assert!(
            note.contains("Data > Import — worked 1× (in 16.7)"),
            "{note}"
        );
        // It failed the last time: no longer offered as what works.
        record_in(
            &dir,
            &outcome(
                "Microsoft Excel",
                Method::Ax,
                "click AXButton “Save”",
                false,
            ),
            3,
        );
        let note = note_in(&dir, "excel", &[]).unwrap();
        assert!(!note.contains("“Save” — worked"), "{note}");
        assert!(
            note.contains("accessibility click AXButton “Save” — failed 1×"),
            "{note}"
        );
        assert!(!names_app("Excellent work", "Microsoft Excel"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_secret_is_kept() {
        let dir = folder("secret");
        assert!(!record_in(
            &dir,
            &outcome(
                "Mail",
                Method::Ax,
                "set_value AXSecureTextField “Password”",
                true
            ),
            1
        ));
        assert!(!record_in(
            &dir,
            &outcome("Safari", Method::Url, "token sk-abc123", true),
            1
        ));
        assert!(record_in(
            &dir,
            &outcome(
                "Safari",
                Method::Url,
                "https://shop.example/checkout?session=abc123&card=4111",
                true
            ),
            1
        ));
        let memory = load(&dir, "Safari").unwrap();
        assert_eq!(memory.entries[0].detail, "https://shop.example/checkout");
        let mut failed = outcome("Mail", Method::Script, "AppleScript", false);
        failed.why = Some("wrong password for max@example.com".into());
        assert!(record_in(&dir, &failed, 2));
        assert_eq!(
            load(&dir, "Mail").unwrap().entries[0].why,
            None,
            "a secret reason is dropped"
        );
        assert!(load(&dir, "Mail").unwrap().entries.len() == 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_job_s_journal_lines_become_procedures() {
        let journal = "# Journal\n- 2026-11-13 10:00 hex: started: budget 1 h\n- worked in Numbers: menu · File > Export To > CSV\n- failed in Safari: visual · click “Login” · button moved\n- learned: something else\n- worked in Keynote: AppleScript\n";
        let found = from_journal(journal, Some("Kalibermarkt"));
        assert_eq!(found.len(), 3);
        assert_eq!(
            (
                found[0].app.as_str(),
                found[0].method,
                found[0].detail.as_str(),
                found[0].ok
            ),
            ("Numbers", Method::Menu, "File > Export To > CSV", true)
        );
        assert_eq!(found[1].method, Method::Visual);
        assert_eq!(found[1].detail, "click “Login”");
        assert_eq!(found[1].why.as_deref(), Some("button moved"));
        assert_eq!(
            (found[2].method, found[2].detail.as_str()),
            (Method::Script, "AppleScript")
        );
        assert_eq!(found[2].project.as_deref(), Some("Kalibermarkt"));
        let dir = folder("journal");
        for outcome in &found {
            record_in(&dir, outcome, 1);
        }
        assert_eq!(
            load(&dir, "Numbers").unwrap().entries[0].project.as_deref(),
            Some("Kalibermarkt")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
