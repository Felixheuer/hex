//! The running tasks: a quiet "N running" with Stop all in the header, and
//! a strip of every task over the list, each with its state (running,
//! taken over, waiting on a usage limit, queued), Take over or Hand back,
//! and Stop.

use rune::{El, Tx, el, icon, row, text};

use super::super::controls::{ButtonKind, Paint, button, pulsing_dot, status_dot};
use super::super::icons::{HAND, TIMER};
use super::super::list_detail::{self as ld, HANG};
use super::super::lists::small_button;
use super::{Hold, Ui};
use crate::assistant::{TaskState, TaskStatus};
use crate::desktop::i18n::{tr, tr_fill};

static STOP_ICON: &rune::Icon = &rune_theme_opencode::icons::STOP;
static ASKING_ICON: &rune::Icon = &rune_theme_opencode::icons::HELP;

/// "3 running" and Stop all, while any task runs, waits, or is queued.
pub(super) fn header_running(ui: &Ui, p: Paint) -> Vec<El> {
    let count = ui.chats.tasks.len();
    if count == 0 {
        return Vec::new();
    }
    let shortcut = ui
        .settings
        .assistant
        .stop_all_hotkey
        .as_ref()
        .map(|binding| super::super::settings::shortcut_caps(binding, false).join(""));
    let running = tr_fill("{} running", &count.to_string());
    let indicator = row()
        .key("chats-running")
        .role("status")
        .label(running.clone())
        .gap(6.0)
        .px(4.0)
        .center_y()
        .child(
            el().key("dot")
                .size(8.0, 8.0)
                .child(pulsing_dot(p.accent()))
                .a11y_hidden(),
        )
        .child(
            text(running)
                .key("count")
                .font(p.k.ty.small_medium)
                .color(p.muted())
                .a11y_hidden(),
        );
    let stop_all = button(
        p,
        "stop-all-tasks",
        tr("Stop all"),
        ButtonKind::Secondary,
        |ui: &mut Ui, _| ui.chats.stop_all(),
    );
    let stop_all = match shortcut {
        Some(keys) => stop_all.help(tr_fill("Stop every task ({})", &keys)),
        None => stop_all,
    };
    vec![indicator, stop_all]
}

/// Every task, each with what it's doing and its actions; none while
/// nothing runs.
pub(super) fn task_strip(ui: &Ui, p: Paint) -> Option<El> {
    let tasks = &ui.chats.tasks;
    if tasks.is_empty() {
        return None;
    }
    let shown = strip_order(tasks, |task| ui.chats.hold(task).is_some());
    let more = tasks.len().saturating_sub(shown.len());
    let c = &p.k.c;
    Some(
        row()
            .key("chats-tasks")
            .role("list")
            .label(tr("Running tasks"))
            .px(HANG)
            .pb(12.0)
            .gap(8.0)
            .flex_wrap()
            .center_y()
            .children(
                shown
                    .into_iter()
                    .enumerate()
                    .map(|(index, task)| chip(ui, p, index, task)),
            )
            // Fifty jobs don't push the conversations off the window: the
            // rest are a press away in the orb's list.
            .children((more > 0).then(|| {
                row()
                    .key("tasks-more")
                    .role("button")
                    .label(tr_fill("{} more tasks", &more.to_string()))
                    .h(30.0)
                    .px(10.0)
                    .center_y()
                    .radius(p.k.radius.md)
                    .fill(c.layer_01)
                    .border(p.hair, c.border_weak)
                    .hover(c.hover)
                    .on_press(|_: &mut Ui| crate::rune_ui::assistant_overlay::toggle_tasks())
                    .child(
                        text(format!("+{more}"))
                            .font(p.k.ty.small_medium)
                            .color(p.muted()),
                    )
            })),
    )
}

/// The chips the strip shows: what waits on the person first, then the
/// rest in order, at most [`STRIP_MAX`].
fn strip_order(tasks: &[TaskStatus], held: impl Fn(&TaskStatus) -> bool) -> Vec<&TaskStatus> {
    let mut ordered: Vec<&TaskStatus> = tasks.iter().filter(|task| held(task)).collect();
    ordered.extend(tasks.iter().filter(|task| !held(task)));
    // A steer queues the conversation's next run beside the one it stops:
    // one chip each.
    let mut seen = std::collections::HashSet::new();
    ordered.retain(|task| seen.insert(task.conversation.clone()));
    ordered.truncate(STRIP_MAX);
    ordered
}

/// The most task chips the strip shows (about two rows).
const STRIP_MAX: usize = 8;

/// What a task is doing, for its chip and its list row: the words, and
/// their ink when it's a state rather than a step.
pub(super) fn doing(ui: &Ui, p: Paint, task: &TaskStatus) -> (String, Option<rune::Color>) {
    match ui.chats.hold(task) {
        Some(Hold::Asking(_)) => (tr("Needs your answer").to_owned(), Some(p.accent())),
        Some(Hold::Paused) => (tr("You have it").to_owned(), Some(p.warning())),
        Some(Hold::Waiting { until_ms, reason }) => {
            let left = countdown(until_ms, crate::history::now_ms());
            let retry = tr_fill("Retries in {}", &left);
            let words = if reason.is_empty() {
                retry
            } else {
                format!("{retry} · {reason}")
            };
            (words, Some(p.muted()))
        }
        None if task.state == TaskState::Queued => (tr("Queued").to_owned(), None),
        None if ui.chats.hands_off(&task.conversation) => (
            tr("Hands off: it's using your pointer").to_owned(),
            Some(p.warning()),
        ),
        None => (
            task.step
                .clone()
                .unwrap_or_else(|| tr("Working…").to_owned()),
            None,
        ),
    }
}

/// `m:ss` until `until_ms`.
pub(super) fn countdown(until_ms: u64, now_ms: u64) -> String {
    let seconds = until_ms.saturating_sub(now_ms).div_ceil(1_000);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A task's mark: its colour pulsing while it runs, a hand while the
/// person has it, a timer while it waits, a quiet dot while queued.
pub(super) fn mark(ui: &Ui, p: Paint, task: &TaskStatus) -> El {
    let colour = super::super::assistant_overlay::task_colour(task.colour, p.k.dark);
    let mark = match ui.chats.hold(task) {
        Some(Hold::Asking(_)) => icon(ASKING_ICON, 13.0).color(p.accent()),
        Some(Hold::Paused) => icon(&HAND, 13.0).color(p.warning()),
        Some(Hold::Waiting { .. }) => icon(&TIMER, 13.0).color(p.muted()),
        None if task.state == TaskState::Queued => status_dot(p.faint()),
        None => pulsing_dot(colour),
    };
    el().key("mark")
        .size(14.0, 14.0)
        .center()
        .child(mark)
        .a11y_hidden()
}

/// One task: its mark, question, and what it's doing, with Take over or
/// Hand back, and Stop.
fn chip(ui: &Ui, p: Paint, _index: usize, task: &TaskStatus) -> El {
    let c = &p.k.c;
    let (detail, ink) = doing(ui, p, task);
    let hold = ui.chats.hold(task);
    let open = task.conversation.clone();
    let stop = task.conversation.clone();
    let id = task.conversation.clone();
    let hand = match hold {
        Some(Hold::Paused) => Some(small_button(
            p,
            "hand-back",
            tr("Hand back"),
            move |ui: &mut Ui, _| ui.chats.hand_back(&id),
        )),
        None if task.state == TaskState::Running => Some(ld::ghost_icon(
            p,
            "take-over",
            &HAND,
            tr("Take over"),
            move |ui: &mut Ui, _| ui.chats.take_over(&id),
        )),
        _ => None,
    };
    let selected = ui.chats.selected.as_ref() == Some(&task.conversation);
    row()
        // Keyed by its conversation, so a chip's hover and motion stay
        // its own when another task ends.
        .key(format!("task-{}", task.conversation))
        .role("listitem")
        .label(task.title.clone())
        .description(detail.clone())
        .h(30.0)
        .pl(8.0)
        .pr(2.0)
        .gap(8.0)
        .center_y()
        .max_w(330.0)
        .radius(p.k.radius.md)
        .fill(if selected { c.layer_02 } else { c.layer_01 })
        .border(
            p.hair,
            if hold == Some(Hold::Paused) {
                c.warning.border
            } else {
                c.border_weak
            },
        )
        .hover(c.hover)
        .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
            tx.instant();
            ui.chats.pick(open.clone());
        })
        .child(mark(ui, p, task))
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
                .color(ink.unwrap_or(p.muted()))
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
        .children(hand)
        .child(ld::ghost_icon(
            p,
            "stop-task",
            STOP_ICON,
            tr("Stop task"),
            move |ui: &mut Ui, _| ui.chats.stop(&stop),
        ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_countdown_reads_minutes_and_seconds() {
        assert_eq!(super::countdown(192_000, 0), "3:12");
        assert_eq!(super::countdown(500, 0), "0:01");
        assert_eq!(super::countdown(0, 10), "0:00");
    }
}

#[cfg(test)]
mod strip_tests {
    use super::*;
    use crate::assistant::Origin;

    fn task(id: &str) -> TaskStatus {
        TaskStatus {
            conversation: id.into(),
            origin: Origin::Background,
            title: id.into(),
            state: TaskState::Running,
            step: None,
            windows: Vec::new(),
            colour: 0,
        }
    }

    #[test]
    fn fifty_tasks_show_eight_chips_what_waits_first() {
        let tasks: Vec<TaskStatus> = (0..50).map(|n| task(&format!("t{n}"))).collect();
        let shown = strip_order(&tasks, |task| task.conversation == "t42");
        assert_eq!(shown.len(), STRIP_MAX);
        assert_eq!(
            shown[0].conversation, "t42",
            "the one waiting on the person"
        );
        assert_eq!(shown[1].conversation, "t0");
        assert_eq!(strip_order(&tasks[..3], |_| false).len(), 3);
    }
}
