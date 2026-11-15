//! A job's terminal, seen without opening anything: a live "Terminal" mini
//! in its card's filmstrip, beside its browser and app minis, for any job
//! that runs commands, searches, reads, or edits files (no special case
//! for coding jobs; `assistant::view::console`). Dark and monospace in
//! both appearances: `$ cargo test`, the dimmed tail of its output,
//! `✎ edited x.rs (+12 −3)` with a few lines of the change in green and
//! red, `⌕ searching "…"`, `▤ reading …`; a blinking cursor and the time
//! while a command runs. A press opens it large in the enlarged card
//! ([`super::enlarged`]): its whole log, newest at the bottom, sticking
//! there as lines come.
//!
//! It draws what the shared view holds: it changes when the task's events
//! do, with no polling of its own.

use std::time::Instant;

use rune::{Color, El, TextStyle, col, el, row, text};
use rune_kit::{IconButton, IconButtonVariant};

use super::super::controls::Paint;
use super::board::Task;
use super::jobs::{THUMB_H, THUMB_W};
use super::{look, palette, stack};
use crate::assistant::view::{Console, Ink, SHOW_ELAPSED, TermLine, ToolKind, elapsed};
use crate::desktop::i18n::{tr, tr_fill};

/// The session a terminal opens as in the enlarged card (a browser's is
/// its own id, an app window's `app_live`'s).
pub(crate) const SESSION: &str = "terminal";
/// The rows a mini holds (its lines and, while a command runs, the
/// cursor), the newest at its bottom: as many as fit its height.
pub(crate) const MINI_LINES: usize = 9;
/// A mini's line: 7.5 on 9.
const MINI_LINE_H: f32 = 9.0;
/// Room inside a mini.
const MINI_PAD: f32 = 6.0;
/// The large view's width.
pub(crate) const LARGE_W: f32 = 720.0;

/// Whether `task` has a terminal to show: it ran, searched, read, or
/// edited something.
pub(crate) fn shows(task: &Task) -> bool {
    !task.console.is_empty()
}

/// The command it runs now and since when, for the cursor and the time.
pub(crate) fn running(task: &Task) -> Option<Instant> {
    task.busy
        .tool()
        .filter(|(tool, _)| tool.kind == ToolKind::Command)
        .map(|(_, since)| since)
}

/// The terminal's own colours, the same in both appearances.
struct Inks {
    bg: Color,
    text: Color,
    dim: Color,
    prompt: Color,
    added: Color,
    removed: Color,
    mark: Color,
}

fn inks() -> Inks {
    Inks {
        bg: rune::color::hex("#111418").unwrap_or(Color::BLACK),
        text: Color::WHITE.alpha(0.9),
        dim: Color::WHITE.alpha(0.5),
        prompt: rune::oklch(0.8, 0.14, 150.0),
        added: rune::oklch(0.78, 0.17, 145.0),
        removed: rune::oklch(0.7, 0.19, 25.0),
        mark: rune::oklch(0.78, 0.12, 250.0),
    }
}

/// One line as the terminal shows it: its words (in the person's language
/// where they are the terminal's own) and its ink.
pub(crate) fn shown(line: &TermLine) -> (String, Ink) {
    let words = match &line.ink {
        Ink::Prompt => format!("$ {}", line.text),
        Ink::Output => line.text.clone(),
        Ink::Edited { added, removed } => {
            let counts = match (added, removed) {
                (Some(added), Some(removed)) => format!(" (+{added} −{removed})"),
                _ => String::new(),
            };
            format!("✎ {}{counts}", tr_fill("edited {}", &line.text))
        }
        Ink::Added => format!("+ {}", line.text),
        Ink::Removed => format!("− {}", line.text),
        Ink::Searched => format!(
            "⌕ {}",
            tr_fill("searching {}", &format!("\"{}\"", line.text))
        ),
        Ink::Read => format!("▤ {}", tr_fill("reading {}", &line.text)),
    };
    (words, line.ink.clone())
}

fn ink_of(ink: &Ink, inks: &Inks) -> Color {
    match ink {
        Ink::Prompt => inks.prompt,
        Ink::Output => inks.dim,
        Ink::Edited { .. } | Ink::Searched | Ink::Read => inks.mark,
        Ink::Added => inks.added,
        Ink::Removed => inks.removed,
    }
}

fn mono(p: Paint, size: f32, line_height: f32) -> TextStyle {
    TextStyle {
        size,
        line_height,
        ..p.k.ty.mono
    }
}

/// The lines as rows of monospace text, and, while a command runs, the
/// cursor (blinking, unless `still`) and its time after the last.
fn lines(
    lines: &[&TermLine],
    running: Option<Instant>,
    style: TextStyle,
    still: bool,
    now: Instant,
) -> Vec<El> {
    let inks = inks();
    let mut rows: Vec<El> = lines
        .iter()
        .enumerate()
        .map(|(at, line)| {
            let (words, ink) = shown(line);
            text(words)
                .key(format!("term-line-{at}"))
                .font(style)
                .color(ink_of(&ink, &inks))
                .truncate()
                .min_w(0.0)
                .a11y_hidden()
        })
        .collect();
    if let Some(since) = running {
        let cursor = el()
            .key("term-cursor")
            .size(style.size * 0.55, style.line_height * 0.85)
            .fill(inks.text)
            .a11y_hidden();
        let cursor = if still {
            cursor
        } else {
            // Lit first, then blinking.
            cursor.looping(rune_motion::LoopSpec::opacity(1.0, 0.0, 1.0, 0.0))
        };
        // Its time once it runs long, as the island's line says it.
        let ran = now.saturating_duration_since(since);
        let time = (ran >= SHOW_ELAPSED).then(|| {
            text(elapsed(ran))
                .key("term-time")
                .font(style)
                .color(inks.dim)
                .a11y_hidden()
        });
        rows.push(
            row()
                .key("term-running")
                .gap(6.0)
                .center_y()
                .child(cursor)
                .children(time),
        );
    }
    rows
}

/// What a screen reader says for the terminal: its latest line.
fn spoken(console: &Console) -> String {
    let last = console.tail(1).first().map(|line| shown(line).0);
    match last {
        Some(last) => format!("{}: {last}", tr("Terminal")),
        None => tr("Terminal").to_owned(),
    }
}

/// The Terminal mini on a job's card: its last [`MINI_LINES`] lines,
/// newest at the bottom, the cursor while a command runs; "Terminal"
/// under it. A press opens it large.
pub(crate) fn mini<S: 'static>(
    p: Paint,
    task: &Task,
    still: bool,
    actions: stack::Actions<S>,
    now: Instant,
) -> El {
    let inks = inks();
    let running = running(task);
    let tail = task
        .console
        .tail(MINI_LINES - usize::from(running.is_some()));
    let screen = col()
        .key(format!("job-term-{}", task.id))
        .size(THUMB_W, THUMB_H)
        .padding(MINI_PAD)
        .gap(0.0)
        .justify_end()
        .radius(8.0)
        .clip_rounded(8.0)
        .fill(inks.bg)
        .border(p.hair, Color::WHITE.alpha(0.12))
        .children(lines(&tail, running, mono(p, 7.5, MINI_LINE_H), still, now));
    let open = actions.open_preview;
    let id = task.id.clone();
    let cell = col()
        .key(format!("job-term-cell-{}", task.id))
        .w(THUMB_W)
        .gap(5.0)
        .child(screen)
        .child(
            text(tr("Terminal"))
                .key("job-term-name")
                .truncate()
                .font(look::meta(p))
                .color(p.muted())
                .min_w(0.0),
        );
    p.k.interactive(
        cell.role("button")
            .label(spoken(&task.console))
            .hit()
            .on_press(move |s: &mut S| open(s, &id, SESSION)),
    )
}

/// What the large terminal shows: the job's title and colour, its log,
/// and the command it runs now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Large {
    pub(crate) title: String,
    pub(crate) colour: u8,
    pub(crate) console: std::sync::Arc<Console>,
    pub(crate) running: Option<Instant>,
}

impl Large {
    pub(crate) fn of(task: &Task) -> Self {
        Self {
            title: task.title.clone(),
            colour: task.colour,
            console: task.console.clone(),
            running: running(task),
        }
    }
}

/// The terminal large, `room` tall at most: "Terminal · <job>" and ×
/// above its whole log, which scrolls and sticks to its newest line.
pub(crate) fn large<S: 'static>(
    p: Paint,
    shown: &Large,
    room: (f32, f32),
    close: fn(&mut S),
    still: bool,
    now: Instant,
) -> El {
    let k = p.k;
    let inks = inks();
    let all: Vec<&TermLine> = shown.console.lines().collect();
    let header = row()
        .key("term-header")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            el().size(8.0, 8.0)
                .radius(4.0)
                .fill(palette::task(shown.colour, k.dark))
                .a11y_hidden(),
        )
        .child(
            text(tr_fill("Terminal · {}", &shown.title))
                .key("term-title")
                .role("heading")
                .truncate()
                .grow(1.0)
                .min_w(0.0)
                .font(k.ty.ui_medium)
                .color(p.text()),
        )
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .label(tr("Close"))
                .on_press(move |s: &mut S| close(s))
                .build(k)
                .key("term-close"),
        );
    let log = super::wheel::scroller::<S>(
        col(),
        super::wheel::Surface::Enlarged,
        "term-log",
        super::wheel::Axis::Y,
    )
    .key("term-log")
    .w_full()
    .h(room.1)
    .padding(12.0)
    .radius(10.0)
    .fill(inks.bg)
    .scroll(rune::ScrollSpec { stick_bottom: true })
    .hit()
    .role("log")
    .label(spoken(&shown.console))
    .children(lines(&all, shown.running, mono(p, 12.0, 17.0), still, now));
    col()
        .key("term-card")
        .role("dialog")
        .label(tr("Terminal"))
        .w(room.0.min(LARGE_W) + 2.0 * super::live_card::PAD)
        .padding(super::live_card::PAD)
        .gap(12.0)
        .radius(k.radius.lg + 4.0)
        .fill(k.c.bg_base)
        .border(p.hair, k.c.border_weak)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 12.0,
            blur: 36.0,
            spread: 0.0,
            color: Color::BLACK.alpha(if k.dark { 0.5 } else { 0.18 }),
            inset: false,
        })
        .hit()
        .on_press(|_: &mut S| {})
        .child(header)
        .child(log)
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod terminal_tests;
