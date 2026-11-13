//! What finished jobs leave at the orb, so their results never need the
//! window: a result bubble (✓ with the answer's first lines, ✗ with why)
//! that pops from the orb and fades, with what the answer names to open
//! (its folder, a file, a link) and Try again on a failure; pressing More
//! shows the whole answer in the bubble and keeps it until it's closed.
//!
//! The board keeps the last few results ([`RECENT_MAX`]) for
//! [`RECALL_FOR`]: the orb stays (quietly) while it has one, and its list
//! shows them under Done, a press bringing one back as its bubble.
//!
//! [`targets`] reads what an answer names (Markdown links, bare addresses,
//! absolute or `~/` paths, and the runner's "Files: <folder>" line) as plain
//! rules, so they're unit tests.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rune::{El, ScrollSpec, col, el, icon, row, text};

use super::super::controls::Paint;
use super::super::icons::{BOOK, FOLDER, GLOBE};
use super::board::{Task, first_line, plain};
use super::look::{self, Card, State};
use super::palette;
use crate::assistant::ConversationId;
use crate::desktop::i18n::{tr, tr_fill};

/// How long a finished job's result can be recalled from the orb.
pub(crate) const RECALL_FOR: Duration = Duration::from_secs(20 * 60);
/// The most results kept.
pub(crate) const RECENT_MAX: usize = 8;
/// The most things a result offers to open.
pub(crate) const MAX_TARGETS: usize = 3;
/// A result bubble's width (the orb's bubbles and job rows share it).
pub(crate) const WIDTH: f32 = look::RAIL_W;
/// The room a folded result bubble takes at most: its head, two lines, and
/// its buttons.
pub(crate) const ROOM: f32 = 170.0;
/// The room an unfolded one takes at most.
pub(crate) const OPEN_ROOM: f32 = 420.0;
/// The unfolded answer's tallest; it scrolls past it.
const ANSWER_MAX_H: f32 = 220.0;
/// The folded answer's most lines.
const LINES: usize = 2;

/// One finished job, as the orb recalls it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) id: ConversationId,
    pub(crate) title: String,
    pub(crate) colour: u8,
    /// It failed: `text` says why.
    pub(crate) failed: bool,
    /// The whole answer (Markdown), or the error.
    pub(crate) text: String,
    pub(crate) at: Instant,
}

impl Outcome {
    /// `task` as it ended (Done or Failed), at `now`.
    pub(crate) fn of(task: &Task, failed: bool, now: Instant) -> Self {
        Self {
            id: task.id.clone(),
            title: task.title.clone(),
            colour: task.colour,
            failed,
            text: if failed {
                task.error.clone().unwrap_or_default()
            } else {
                task.answer.clone()
            },
            at: now,
        }
    }

    /// Its first line, as a person reads it (no Markdown marks).
    pub(crate) fn summary(&self) -> String {
        let line = first_line(&self.text);
        if line.is_empty() {
            self.title.clone()
        } else {
            line
        }
    }

    /// What it names to open.
    pub(crate) fn targets(&self) -> Vec<Target> {
        if self.failed {
            Vec::new()
        } else {
            targets(&self.text)
        }
    }
}

/// What a result offers to open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Folder,
    File,
    Link,
}

/// One thing an answer names: a folder, a file, or a link.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Target {
    pub(crate) kind: Kind,
    /// What `open` takes: the path (with `~` expanded) or the address.
    pub(crate) open: String,
    /// What the button says: the folder's or file's name, or the site.
    pub(crate) name: String,
}

impl Target {
    /// What a screen reader says for its button.
    pub(crate) fn label(&self) -> String {
        match self.kind {
            Kind::Folder => tr_fill("Open folder {}", &self.name),
            Kind::File => tr_fill("Open file {}", &self.name),
            Kind::Link => tr_fill("Open link {}", &self.name),
        }
    }
}

/// What `answer` names to open, as the file system says each path is.
pub(crate) fn targets(answer: &str) -> Vec<Target> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    targets_with(answer, home.as_deref(), |path| {
        std::fs::metadata(path).ok().map(|meta| meta.is_dir())
    })
}

/// [`targets`] with the home folder and the file system given: `is_dir`
/// says whether a path is a folder, `None` when it isn't there (then a name
/// with an extension is a file). At most [`MAX_TARGETS`], each once, in the
/// order the answer names them.
pub(crate) fn targets_with(
    answer: &str,
    home: Option<&Path>,
    is_dir: impl Fn(&Path) -> Option<bool>,
) -> Vec<Target> {
    let mut found: Vec<Target> = Vec::new();
    let mut add = |target: Option<Target>| {
        if let Some(target) = target
            && !found.iter().any(|known| known.open == target.open)
        {
            found.push(target);
        }
    };
    for line in answer.lines() {
        let line = line.trim();
        // The runner's own line: the whole rest is the folder, spaces and all.
        if let Some(rest) = line.strip_prefix("Files:") {
            add(path_target(rest.trim().trim_matches('`'), home, &is_dir));
            continue;
        }
        // Markdown links: their address.
        let mut rest = line;
        while let Some(open) = rest.find("](") {
            let after = &rest[open + 2..];
            let Some(close) = after.find(')') else {
                break;
            };
            add(any_target(&after[..close], home, &is_dir));
            rest = &after[close + 1..];
        }
        // Code spans: a path with spaces in it.
        for span in line.split('`').skip(1).step_by(2) {
            if span.starts_with('/') || span.starts_with("~/") {
                add(path_target(span, home, &is_dir));
            }
        }
        // Bare words outside code spans: addresses and paths.
        let outside: Vec<&str> = line.split('`').step_by(2).collect();
        let outside = outside.join(" ");
        for word in outside.split(|c: char| c.is_whitespace() || c == '(' || c == '<') {
            let word = word
                .trim_end_matches(['.', ',', ';', ':', ')', ']', '>', '"', '\'', '!', '?', '*']);
            let word = word.trim_start_matches(['"', '\'', '*', '[']);
            if word.starts_with("http://") || word.starts_with("https://") {
                add(any_target(word, home, &is_dir));
            } else if looks_like_path(word) {
                add(path_target(word, home, &is_dir));
            }
        }
    }
    found.truncate(MAX_TARGETS);
    found
}

/// An absolute path somewhere a person keeps things, or one in their home.
fn looks_like_path(word: &str) -> bool {
    const ROOTS: [&str; 6] = [
        "/Users/",
        "/Volumes/",
        "/Applications/",
        "/tmp/",
        "/private/",
        "/opt/",
    ];
    word.starts_with("~/")
        || ROOTS
            .iter()
            .any(|root| word.starts_with(root) && word.len() > root.len())
}

fn any_target(
    address: &str,
    home: Option<&Path>,
    is_dir: &impl Fn(&Path) -> Option<bool>,
) -> Option<Target> {
    let address = address.trim();
    if address.starts_with("http://") || address.starts_with("https://") {
        let name = super::live::host_of(address).unwrap_or_else(|| address.to_owned());
        return Some(Target {
            kind: Kind::Link,
            open: address.to_owned(),
            name,
        });
    }
    let path = address.strip_prefix("file://").unwrap_or(address);
    if path.starts_with('/') || path.starts_with("~/") {
        return path_target(path, home, is_dir);
    }
    None
}

fn path_target(
    path: &str,
    home: Option<&Path>,
    is_dir: &impl Fn(&Path) -> Option<bool>,
) -> Option<Target> {
    let path = path.trim().trim_end_matches(['.', ',', ';', ':']);
    if path.is_empty() || path == "/" || path == "~/" {
        return None;
    }
    let full: PathBuf = match path.strip_prefix("~/") {
        Some(rest) => home?.join(rest),
        None => PathBuf::from(path),
    };
    let trimmed = full.to_string_lossy().trim_end_matches('/').to_owned();
    let full = PathBuf::from(&trimmed);
    let name = full
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| trimmed.clone());
    let folder = is_dir(&full).unwrap_or_else(|| full.extension().is_none());
    Some(Target {
        kind: if folder { Kind::Folder } else { Kind::File },
        open: trimmed,
        name,
    })
}

/// Opens `target` (a path or an address) as Finder or the browser would,
/// off the caller's thread.
pub(crate) fn open(target: &str) {
    let target = target.to_owned();
    let _ = std::thread::Builder::new()
        .name("hex-assistant-open".into())
        .spawn(move || {
            if let Err(error) = crate::desktop::opencode_catalog::open_url(&target) {
                tracing::warn!(%error, "assistant: couldn't open a job's result");
            }
        });
}

/// What a result's buttons ask of the host.
pub(crate) struct Actions<S> {
    /// A folder, file, or link it names (what `open` takes).
    pub(crate) open: fn(&mut S, &str),
    /// Try again: the failed job runs once more (its id).
    pub(crate) retry: fn(&mut S, &str),
    /// × on its bubble (the bubble's key).
    pub(crate) dismiss: fn(&mut S, u64),
    /// More / Less on its bubble (the bubble's key).
    pub(crate) expand: fn(&mut S, u64),
    /// A result in the list: its bubble again, unfolded (its id).
    pub(crate) recall: fn(&mut S, &str),
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

/// A result's card in the rail, keyed by its bubble's `key`, on the one
/// anatomy (`look::card`): the job's dot and title with its Done or Failed
/// chip; its answer's first lines (the whole answer, scrolling, with every
/// target as a row, when `open`), or why it failed; and its own actions:
/// the first thing it names to open as the primary (Try again on a
/// failure), the second and More as the secondary ones. A press on the
/// card folds or unfolds it.
pub(crate) fn bubble<S: 'static>(
    p: Paint,
    key: u64,
    outcome: &Outcome,
    open: bool,
    actions: Actions<S>,
) -> El {
    let colour = palette::task(outcome.colour, p.k.dark);
    let expand = actions.expand;
    let state = state(outcome);
    let long = outcome
        .text
        .trim()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count()
        > LINES
        || outcome.text.chars().count() > 90;
    let targets = outcome.targets();
    let mut body: Vec<El> = Vec::new();
    if open && !outcome.failed {
        body.push(
            el().key("result-answer")
                .w_full()
                .max_h(ANSWER_MAX_H)
                .min_h(0.0)
                .shrink()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .scroll_edge_fade(12.0)
                .child(super::markdown::view(p, outcome.text.trim())),
        );
        // Every target as a row with its verb.
        body.extend(
            targets
                .iter()
                .enumerate()
                .map(|(index, target)| target_row(p, index, target, actions)),
        );
    } else {
        body.push(
            look::body_text(
                p,
                &if open {
                    outcome.text.trim().to_owned()
                } else {
                    summary_lines(&outcome.text, outcome.failed)
                },
            )
            .key("result-text")
            .max_lines(if open { 12 } else { LINES }),
        );
    }
    let mut card = Card::named("result");
    card.lead = Some(look::dot(colour, 9.0));
    card.title = outcome.title.clone();
    card.chip = Some(look::chip(p, state).key(state.key()));
    card.body = body;
    card.width = Some(WIDTH);
    if outcome.failed {
        let retry = actions.retry;
        let id = outcome.id.clone();
        card.primary = Some(
            look::button(p, look::Kind::Primary, tr("Try again"), move |s: &mut S| {
                retry(s, &id)
            })
            .key("result-retry"),
        );
    } else if !open {
        let mut shown = targets.iter().enumerate();
        card.primary = shown
            .next()
            .map(|(index, target)| target_button(p, look::Kind::Primary, index, target, actions));
        card.secondary.extend(shown.next().map(|(index, target)| {
            target_button(p, look::Kind::Secondary, index, target, actions)
        }));
    }
    if long {
        card.secondary.push(
            look::button(
                p,
                look::Kind::Secondary,
                if open { tr("Less") } else { tr("More") },
                move |s: &mut S| expand(s, key),
            )
            .key("result-more"),
        );
    }
    p.k.interactive(
        look::card(p, card)
            .key(format!("result-{key}"))
            .role("status")
            .label(format!(
                "{}: {} · {}",
                state.word(),
                outcome.title,
                outcome.summary()
            ))
            .live_region(rune::LiveRegion::Polite)
            // The whole card takes the pointer, so its buttons, the wheel
            // over a long answer, and the space between them answer.
            .hit()
            .on_press(move |s: &mut S| expand(s, key)),
    )
}

/// Done or Failed.
fn state(outcome: &Outcome) -> State {
    if outcome.failed {
        State::Failed
    } else {
        State::Done
    }
}

/// A result in the list's Done group: its dot, title, and first line, its
/// Done or Failed chip, and its own actions (what it names to open, or Try
/// again); a press brings its card back.
pub(crate) fn list_row<S: 'static>(p: Paint, outcome: &Outcome, actions: Actions<S>) -> El {
    let recall = actions.recall;
    let id = outcome.id.clone();
    let mut buttons: Vec<El> = Vec::new();
    if outcome.failed {
        let retry = actions.retry;
        let id = outcome.id.clone();
        buttons.push(
            look::button(p, look::Kind::Primary, tr("Try again"), move |s: &mut S| {
                retry(s, &id)
            })
            .key("result-retry"),
        );
    } else {
        buttons.extend(
            outcome
                .targets()
                .iter()
                .enumerate()
                .take(2)
                .map(|(index, target)| {
                    target_button(
                        p,
                        if index == 0 {
                            look::Kind::Primary
                        } else {
                            look::Kind::Secondary
                        },
                        index,
                        target,
                        actions,
                    )
                }),
        );
    }
    let state = state(outcome);
    let head = row()
        .key("head")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(look::dot(palette::task(outcome.colour, p.k.dark), 9.0))
        .child(
            text(outcome.title.clone())
                .key("title")
                .truncate()
                .font(look::label(p))
                .color(p.text())
                .grow(1.0)
                .min_w(0.0),
        )
        .child(look::chip(p, state).key(state.key()));
    let lines = col()
        .key("lines")
        .w_full()
        .gap(4.0)
        .child(head)
        .child(
            text(outcome.summary())
                .key("outcome")
                .truncate()
                .font(look::meta(p))
                .color(p.muted())
                .pl(17.0)
                .min_w(0.0),
        )
        .children((!buttons.is_empty()).then(|| {
            row()
                .key("result-actions")
                .w_full()
                .pt(4.0)
                .pl(17.0)
                .gap(8.0)
                .justify_end()
                .center_y()
                .children(buttons)
        }));
    p.k.interactive(
        col()
            .key(format!("result-row-{}", outcome.id))
            .w_full()
            .px(12.0)
            .py(10.0)
            .radius(12.0)
            .role("listitem")
            .label(format!(
                "{}: {} · {}",
                state.word(),
                outcome.title,
                outcome.summary()
            ))
            .hit()
            .on_press(move |s: &mut S| recall(s, &id))
            .child(lines),
    )
}

/// What a target's glyph is: a folder, a page, or a globe.
fn glyph(kind: Kind) -> &'static rune::Icon {
    match kind {
        Kind::Folder => &FOLDER,
        Kind::File => &BOOK,
        Kind::Link => &GLOBE,
    }
}

/// One thing to open, as a pill button: "Open Angebote".
fn target_button<S: 'static>(
    p: Paint,
    kind: look::Kind,
    index: usize,
    target: &Target,
    actions: Actions<S>,
) -> El {
    let open = actions.open;
    let what = target.open.clone();
    look::button(
        p,
        kind,
        &tr_fill(
            "Open {}",
            &crate::assistant::store::shorten(&target.name, 18),
        ),
        move |s: &mut S| open(s, &what),
    )
    .key(format!("result-open-{index}"))
    .label(target.label())
    .max_w(170.0)
}

/// One thing to open, as a row in an unfolded card: its glyph, its name,
/// and Open.
fn target_row<S: 'static>(p: Paint, index: usize, target: &Target, actions: Actions<S>) -> El {
    let open = actions.open;
    let what = target.open.clone();
    p.k.interactive(
        row()
            .key(format!("result-open-{index}"))
            .w_full()
            .h(30.0)
            .px(8.0)
            .gap(8.0)
            .center_y()
            .radius(8.0)
            .role("button")
            .label(target.label())
            .hit()
            .on_press(move |s: &mut S| open(s, &what))
            .child(
                icon(glyph(target.kind), 14.0)
                    .color(p.muted())
                    .a11y_hidden(),
            )
            .child(
                text(target.name.clone())
                    .truncate()
                    .font(look::label(p))
                    .color(p.text())
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(text(tr("Open")).font(look::meta(p)).color(p.muted())),
    )
}

/// The answer's first [`LINES`] non-empty lines as plain text, or the
/// error's.
fn summary_lines(text: &str, failed: bool) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(plain)
        .filter(|line| !line.is_empty())
        // The runner's folder line has its own button.
        .filter(|line| failed || !line.starts_with("Files:"))
        .take(LINES)
        .collect();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(path: &Path) -> Option<bool> {
        let path = path.to_string_lossy();
        if path.ends_with("Lieferanten") || path.ends_with("Agent-Desk Jobs/Angebote") {
            Some(true)
        } else if path.ends_with(".xlsx") {
            Some(false)
        } else {
            None
        }
    }

    #[test]
    fn an_answer_names_its_folder_files_and_links() {
        let home = Path::new("/Users/maya");
        let answer = "Done: I compared **50 suppliers**. The table is in \
                      `/Users/maya/Agent-Desk Jobs/Angebote/Vergleich.xlsx`, see also \
                      [the shop](https://shop.example.com/preise).\n\nFiles: /Users/maya/Agent-Desk Jobs/Angebote";
        let found = targets_with(answer, Some(home), dirs);
        assert_eq!(
            found,
            [
                Target {
                    kind: Kind::Link,
                    open: "https://shop.example.com/preise".into(),
                    name: "shop.example.com".into(),
                },
                Target {
                    kind: Kind::File,
                    open: "/Users/maya/Agent-Desk Jobs/Angebote/Vergleich.xlsx".into(),
                    name: "Vergleich.xlsx".into(),
                },
                Target {
                    kind: Kind::Folder,
                    open: "/Users/maya/Agent-Desk Jobs/Angebote".into(),
                    name: "Angebote".into(),
                },
            ]
        );
    }

    #[test]
    fn home_paths_expand_and_bare_addresses_lose_their_punctuation() {
        let home = Path::new("/Users/maya");
        let found = targets_with(
            "Saved to ~/Desktop/Lieferanten. Details: https://example.org/a?b=1.",
            Some(home),
            dirs,
        );
        assert_eq!(found[0].open, "/Users/maya/Desktop/Lieferanten");
        assert_eq!(found[0].kind, Kind::Folder);
        assert_eq!(found[1].open, "https://example.org/a?b=1");
        assert_eq!(found[1].kind, Kind::Link);
    }

    #[test]
    fn a_missing_path_is_a_file_by_its_extension_and_each_shows_once() {
        let found = targets_with(
            "Wrote /tmp/report.pdf and /tmp/report.pdf again, into /tmp/out/",
            None,
            |_| None,
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].kind, Kind::File);
        assert_eq!(found[1].kind, Kind::Folder);
        assert_eq!(found[1].name, "out");
        // Nothing to open: nothing offered.
        assert!(targets_with("All done, nothing to file.", None, |_| None).is_empty());
        // At most three.
        let many = "https://a.com https://b.com https://c.com https://d.com";
        assert_eq!(targets_with(many, None, |_| None).len(), MAX_TARGETS);
    }

    #[test]
    fn a_folded_result_reads_two_plain_lines_without_the_folder_line() {
        assert_eq!(
            summary_lines(
                "## **Fertig**\n\n- 50 Lieferanten verglichen\n- Tabelle geschickt\n\nFiles: /x",
                false
            ),
            "Fertig\n50 Lieferanten verglichen"
        );
        assert_eq!(summary_lines("Files: /x", false), "");
    }
}
