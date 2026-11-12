//! The Chats view: the setup notice, the running tasks, the conversations
//! grouped by age, and the selected conversation's transcript.

use rune::{El, Tx, col, el, row, text};
use rune_kit::TextInput;

use super::super::controls::{ButtonKind, DESCRIPTION, Paint, button, pulsing_dot, status_dot};
use super::super::icons::{KEYBOARD, MIC};
use super::super::list_detail::{self as ld, HANG, Item};
use super::super::lists::{dismissible_error_panel, message, small_button};
use super::{Ui, transcript};
use crate::assistant::{ConversationSummary, Origin, TaskState, TaskStatus};
use crate::desktop::i18n::tr;

/// The list's width: narrower than Meetings', so the transcript has room.
const LIST_WIDTH: f32 = 272.0;
const DAY: u64 = 24 * 60 * 60 * 1_000;

pub(super) static AGENT_ICON: &rune::Icon = &rune_theme_opencode::icons::WINDOW_CURSOR;
static STOP_ICON: &rune::Icon = &rune_theme_opencode::icons::STOP;

/// The icon before a conversation: how it was asked.
pub(super) fn origin_icon(origin: Origin) -> &'static rune::Icon {
    match origin {
        Origin::ActionShortcut => &MIC,
        Origin::AgentMode => AGENT_ICON,
        Origin::Chat => &KEYBOARD,
    }
}

/// How it was asked, for the transcript's meta line.
pub(super) fn origin_label(origin: Origin) -> &'static str {
    match origin {
        Origin::ActionShortcut => tr("Asked by voice"),
        Origin::AgentMode => tr("Agent mode task"),
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
        .children(task_strip(ui, p))
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

/// The tasks running or waiting, each with Stop; none while nothing runs.
fn task_strip(ui: &Ui, p: Paint) -> Option<El> {
    let tasks = &ui.chats.tasks;
    if tasks.is_empty() {
        return None;
    }
    let mut strip = row()
        .key("chats-tasks")
        .role("list")
        .label(tr("Running tasks"))
        .px(HANG)
        .pb(12.0)
        .gap(8.0)
        .flex_wrap()
        .center_y()
        .children(
            tasks
                .iter()
                .enumerate()
                .map(|(index, task)| chip(p, index, task)),
        );
    if tasks.len() > 1 {
        strip = strip.child(small_button(
            p,
            "stop-all-tasks",
            tr("Stop all"),
            |ui: &mut Ui, _| ui.chats.stop_all(),
        ));
    }
    Some(strip)
}

/// One task: its state, question, and latest step, with Stop.
fn chip(p: Paint, index: usize, task: &TaskStatus) -> El {
    let c = &p.k.c;
    // The task's own colour, as its cursor, outline, and stack row show it.
    let colour = super::super::assistant_overlay::task_colour(task.colour, p.k.dark);
    let dot = if task.state == TaskState::Running {
        pulsing_dot(colour)
    } else {
        status_dot(p.faint())
    };
    let detail = match (&task.step, task.state) {
        (_, TaskState::Queued) => tr("Waiting for a free slot").to_owned(),
        (Some(step), _) => step.clone(),
        (None, _) => tr("Working…").to_owned(),
    };
    let open = task.conversation.clone();
    let stop = task.conversation.clone();
    row()
        .key(format!("task{index}"))
        .role("listitem")
        .label(task.title.clone())
        .description(detail.clone())
        .h(30.0)
        .pl(10.0)
        .pr(2.0)
        .gap(8.0)
        .center_y()
        .max_w(340.0)
        .radius(p.k.radius.md)
        .fill(c.layer_01)
        .border(p.hair, c.border_weak)
        .hover(c.hover)
        .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
            tx.instant();
            ui.chats.select(open.clone());
        })
        .child(el().key("mark").size(8.0, 8.0).child(dot).a11y_hidden())
        .child(
            text(task.title.clone())
                .key("title")
                .font(p.k.ty.small_medium)
                .color(p.text())
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
        .child(
            text(detail)
                .key("step")
                .font(p.k.ty.small)
                .color(p.muted())
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
        .child(ld::ghost_icon(
            p,
            "stop-task",
            STOP_ICON,
            tr("Stop task"),
            move |ui: &mut Ui, _| ui.chats.stop(&stop),
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

/// One conversation's row: a live dot while it runs, else how it was asked.
fn item(ui: &Ui, p: Paint, summary: &ConversationSummary, now: u64) -> El {
    let pane = &ui.chats;
    let running = pane.running(&summary.id);
    let failed = pane.failures.contains_key(&summary.id);
    let (meta, meta_ink) = if running {
        let step = pane
            .turns
            .get(&summary.id)
            .and_then(|turn| turn.steps.last().cloned())
            .or_else(|| {
                pane.tasks
                    .iter()
                    .find(|task| task.conversation == summary.id)
                    .and_then(|task| task.step.clone())
            })
            .unwrap_or_else(|| tr("Working…").to_owned());
        (step, None)
    } else if failed {
        (tr("Failed").to_owned(), Some(p.danger()))
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
    let select = move |ui: &mut Ui| ui.chats.select(id.clone());
    if running {
        let colour = pane
            .tasks
            .iter()
            .find(|task| task.conversation == summary.id)
            .map_or(p.accent(), |task| {
                super::super::assistant_overlay::task_colour(task.colour, p.k.dark)
            });
        let mark = el()
            .key("live")
            .size(14.0, 14.0)
            .center()
            .child(pulsing_dot(colour));
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
