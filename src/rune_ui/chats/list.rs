//! The Chats view: the setup notice, the running tasks, the conversations
//! grouped by age, and the selected conversation's transcript.

use rune::{El, col, el, icon, row, text};
use rune_kit::TextInput;

use super::super::controls::{ButtonKind, DESCRIPTION, Paint, button, pulsing_dot};
use super::super::icons::{KEYBOARD, MIC, SPARKLES};
use super::super::list_detail::{self as ld, Item};
use super::super::lists::{dismissible_error_panel, message};
use super::{Ui, strip, transcript};
use crate::assistant::{ConversationSummary, Origin, Outcome};
use crate::desktop::i18n::tr;

/// The list's width: narrower than Meetings', so the transcript has room.
const LIST_WIDTH: f32 = 272.0;
const DAY: u64 = 24 * 60 * 60 * 1_000;

/// A step's icon in the transcript: something done on the desktop.
pub(super) static AGENT_ICON: &rune::Icon = &rune_theme_opencode::icons::WINDOW_CURSOR;
static FAILED_ICON: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_X;
pub(super) static INTERRUPTED_ICON: &rune::Icon = &rune_theme_opencode::icons::WARNING;

/// The icon before a conversation: its lane, and how it was asked.
pub(super) fn origin_icon(origin: Origin) -> &'static rune::Icon {
    match origin {
        Origin::ActionShortcut => &MIC,
        Origin::Background => &SPARKLES,
        Origin::Chat => &KEYBOARD,
    }
}

/// How it was asked, for the transcript's meta line.
pub(super) fn origin_label(origin: Origin) -> &'static str {
    match origin {
        Origin::ActionShortcut => tr("Here: about the window in front"),
        Origin::Background => tr("Background task"),
        Origin::Chat => tr("Typed in Chats"),
    }
}

pub(super) fn body(ui: &Ui, p: Paint) -> El {
    let pane = &ui.chats;
    let mut notices = Vec::new();
    if !ui.settings.assistant.setup_done {
        notices.push(setup_notice(p));
    }
    if let Some(error) = &pane.error {
        notices.push(dismissible_error_panel(
            p,
            "chats-error",
            tr("Couldn't send"),
            error,
            |ui: &mut Ui| ui.chats.error = None,
        ));
    }
    let column = ld::list_column_sized(p, search(ui, p), "chats-scroll", list(ui, p), LIST_WIDTH);
    col()
        .key("chats-pane")
        .grow(1.0)
        .shrink()
        .children((!notices.is_empty()).then(|| ld::notices(notices)))
        .children(strip::task_strip(ui, p))
        .child(ld::body(p, "chats-body", column, transcript::detail(ui, p)))
}

fn search(ui: &Ui, p: Paint) -> El {
    TextInput::new(&ui.chats.search)
        .placeholder(tr("Search chats"))
        .width(None)
        .leading_icon(p.k.glyph.search)
        .on_edit(|ui: &mut Ui| &mut ui.chats.search)
        .build(p.k)
        .key("chats-search")
}

/// Finish setting up, until the sheet's Finish.
fn setup_notice(p: Paint) -> El {
    let c = &p.k.c;
    row()
        .key("assistant-setup-notice")
        .pl(12.0)
        .pr(8.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .radius(p.k.radius.md)
        .fill(c.info.bg)
        .border(0.5, c.info.border)
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(
                    text(tr("Finish setting up the assistant"))
                        .key("title")
                        .font(p.k.ty.ui_medium)
                        .color(p.text()),
                )
                .child(
                    text(tr(
                        "Check the agents, agent-desktop, and permissions it needs",
                    ))
                    .key("detail")
                    .font(DESCRIPTION)
                    .color(p.muted())
                    .wrap(),
                ),
        )
        .child(button(
            p,
            "open-assistant-setup",
            tr("Set up…"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| super::open_setup(ui),
        ))
}

/// Which group a conversation lists under, by its last change.
fn group(updated_ms: u64, now_ms: u64) -> &'static str {
    match crate::meeting::day_label(updated_ms, now_ms).as_str() {
        "Today" => tr("Today"),
        "Yesterday" => tr("Yesterday"),
        _ if now_ms.saturating_sub(updated_ms) < 7 * DAY => tr("Previous 7 days"),
        _ => tr("Older"),
    }
}

/// The conversations, grouped by age, newest first.
fn list(ui: &Ui, p: Paint) -> El {
    let pane = &ui.chats;
    let rows = pane.rows();
    let mut list = col().key("chats-list").label(tr("Chats"));
    if rows.is_empty() {
        let empty = if pane.search.text.trim().is_empty() {
            tr("No chats yet.")
        } else {
            tr("No chats match.")
        };
        return list.child(message(p, "chats-none", empty, None));
    }
    let now = crate::history::now_ms();
    let count = rows.len();
    let mut heading: Option<&str> = None;
    for (index, summary) in rows.iter().enumerate() {
        let label = group(summary.updated_ms, now);
        if heading != Some(label) {
            list = list.child(ld::group(
                p,
                &format!("group{index}"),
                label,
                heading.is_none(),
            ));
            heading = Some(label);
        }
        list = list.child(item(ui, p, summary, now).position_in_set(index + 1, count));
    }
    list
}

/// One conversation's row: its task's mark while it runs, a mark for how
/// it ended when that went wrong, else how it was asked.
fn item(ui: &Ui, p: Paint, summary: &ConversationSummary, now: u64) -> El {
    let pane = &ui.chats;
    let running = pane.running(&summary.id);
    let task = pane.task(&summary.id);
    let failed = pane.failures.contains_key(&summary.id)
        || (!running && summary.outcome == Outcome::Failed);
    let interrupted = !running && summary.outcome == Outcome::Interrupted;
    let latest_step = || {
        pane.turns
            .get(&summary.id)
            .and_then(|turn| turn.steps.last().cloned())
    };
    let (meta, meta_ink) = if let Some(task) = task {
        match strip::doing(ui, p, task) {
            // A state (taken over, waiting, queued) says so.
            (words, Some(ink)) => (words, Some(ink)),
            (words, None) => (latest_step().unwrap_or(words), None),
        }
    } else if running {
        let step = latest_step().unwrap_or_else(|| tr("Working…").to_owned());
        (step, None)
    } else if failed {
        (tr("Failed").to_owned(), Some(p.danger()))
    } else if interrupted {
        (
            tr("Interrupted: Hex quit while it ran").to_owned(),
            Some(p.warning()),
        )
    } else if summary.outcome == Outcome::Cancelled {
        (tr("Stopped").to_owned(), Some(p.faint()))
    } else {
        (plain(&summary.preview), None)
    };
    let id = summary.id.clone();
    let entry = Item {
        key: format!("chat-{}", summary.id),
        title: summary.title.clone(),
        meta,
        meta_ink,
        time: if running {
            String::new()
        } else {
            super::age(summary.updated_ms, now)
        },
        hover: Vec::new(),
        selected: pane.selected.as_deref() == Some(summary.id.as_str()),
    };
    let select = move |ui: &mut Ui| ui.chats.pick(id.clone());
    let outcome_mark = |glyph: &'static rune::Icon, ink| {
        el().key("outcome")
            .size(14.0, 14.0)
            .center()
            .child(icon(glyph, 14.0).color(ink))
    };
    if let Some(task) = task {
        ld::item_with_mark(p, entry, strip::mark(ui, p, task), select)
    } else if running {
        let mark = el()
            .key("live")
            .size(14.0, 14.0)
            .center()
            .child(pulsing_dot(p.accent()));
        ld::item_with_mark(p, entry, mark, select)
    } else if failed {
        ld::item_with_mark(p, entry, outcome_mark(FAILED_ICON, p.danger()), select)
    } else if interrupted {
        let mark = outcome_mark(INTERRUPTED_ICON, p.warning());
        ld::item_with_mark(p, entry, mark, select)
    } else {
        ld::item_with_icon(p, entry, Some(origin_icon(summary.origin)), select)
    }
}

/// A preview line without its Markdown marks: `**bold**` reads as bold's
/// words, `# Title` as the title.
pub(super) fn plain(markdown: &str) -> String {
    let line = markdown.trim_start_matches(['#', '>', '-', '*', ' ']);
    line.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
