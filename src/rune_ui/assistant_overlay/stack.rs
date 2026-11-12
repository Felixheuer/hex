//! The expanded task list: a sheet that unfolds from the orb (double-tap
//! the Action shortcut, press the orb, or rest the pointer on it) with every
//! Background task, newest nearest the orb. Each row shows its colour, its
//! question, where it works (a globe for the web, a window for desktop
//! apps, with their names), its last few steps, and its state: Queued,
//! Running, "You have it" (paused), the minutes until a usage limit lifts,
//! ✓, ✗, or Stopped. Take over and Hand back pass a running task between
//! the agent and the person; × stops one, Stop all every one (with two or
//! more); pressing a row opens its conversation in Chats.

use rune::{El, ScrollSpec, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Spinner};

use super::super::controls::Paint;
use super::super::icons::{GLOBE, PANEL};
use super::board::{Phase, Place, Task};
use super::{palette, when};
use crate::desktop::i18n::{tr, tr_fill};

/// The sheet's width.
pub(crate) const WIDTH: f32 = 372.0;
const ROW_GAP: f32 = 2.0;

/// What a press asks of the host.
pub(crate) struct Actions<S> {
    /// × on a task: stop it.
    pub(crate) stop: fn(&mut S, &str),
    /// A row: open its conversation in Chats.
    pub(crate) open: fn(&mut S, &str),
    /// Take over a running task: its desktop actions wait.
    pub(crate) take_over: fn(&mut S, &str),
    /// Hand a taken-over task back.
    pub(crate) hand_back: fn(&mut S, &str),
    /// Stop every task.
    pub(crate) stop_all: fn(&mut S),
    /// Fold the list back into the orb.
    pub(crate) toggle: fn(&mut S),
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

/// The sheet with `tasks` (oldest first), at most `max_h` tall; its rows
/// scroll past that. `newest_first` lists the newest at the top (the sheet
/// hangs below the orb).
pub(crate) fn sheet<S: 'static>(
    p: Paint,
    tasks: &[Task],
    newest_first: bool,
    max_h: f32,
    actions: Actions<S>,
) -> El {
    let c = &p.k.c;
    let live = tasks.iter().filter(|task| task.phase.live()).count();
    let stop_all = (live >= 2).then(|| {
        let stop_all = actions.stop_all;
        Button::new(tr("Stop all"))
            .variant(ButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut S| stop_all(s))
            .build(p.k)
            .key("stop-all")
    });
    let toggle = actions.toggle;
    let header = row()
        .key("sheet-header")
        .w_full()
        .h(32.0)
        .pl(10.0)
        .pr(4.0)
        .gap(4.0)
        .center_y()
        .child(
            text(tr("Assistant tasks"))
                .key("sheet-title")
                .font(p.k.ty.small_medium)
                .color(p.muted())
                .grow(1.0)
                .min_w(0.0),
        )
        .children(stop_all)
        .child(
            IconButton::new(p.k.glyph.chevron_up)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(tr("Hide details"))
                .on_press(move |s: &mut S| toggle(s))
                .build(p.k)
                .key("fold"),
        );
    let mut ordered: Vec<&Task> = tasks.iter().collect();
    if newest_first {
        ordered.reverse();
    }
    let rows = col()
        .key("sheet-rows")
        .w_full()
        .min_h(0.0)
        .shrink()
        .gap(ROW_GAP)
        .scroll(ScrollSpec {
            stick_bottom: !newest_first,
        })
        .scroll_edge_fade(16.0)
        .role("list")
        .label(tr("Assistant tasks"))
        .children(ordered.into_iter().map(|task| task_row(p, task, actions)));
    col()
        .key("sheet")
        .w(WIDTH)
        .max_h(max_h)
        .padding(4.0)
        .radius(p.k.radius.md + 10.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 8.0,
            blur: 28.0,
            spread: 0.0,
            color: rune::Color::BLACK.alpha(if p.k.dark { 0.45 } else { 0.16 }),
            inset: false,
        })
        .hit()
        .animate_layout()
        .child(header)
        .child(rows)
}

fn task_row<S: 'static>(p: Paint, task: &Task, actions: Actions<S>) -> El {
    let colour = palette::task(task.colour, p.k.dark);
    let live = task.phase.live();
    let dot = super::orb::dot(task, colour, 10.0).mt(5.0);
    let title = text(task.title.clone())
        .key("title")
        .truncate()
        .font(p.k.ty.ui_medium)
        .color(if live { p.text() } else { p.muted() });
    let place = place_line(p, task);
    let lines = col()
        .key("lines")
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(title)
        .children(place)
        .children(steps(p, task))
        .children(buttons(p, task, actions));
    let open = actions.open;
    let id = task.id.clone();
    let body = row()
        .key(format!("task-{}", task.id))
        .w_full()
        .pl(10.0)
        .pr(8.0)
        .py(8.0)
        .gap(10.0)
        .items_start()
        .radius(p.k.radius.md + 4.0)
        .role("listitem")
        .label(task.title.clone())
        .hit()
        .on_press(move |s: &mut S| open(s, &id))
        .child(dot)
        .child(lines)
        .child(status(p, task, colour).mt(2.0));
    let body = if matches!(task.phase, Phase::Waiting { .. }) {
        body.opacity(0.6)
    } else {
        body
    };
    p.k.interactive(body)
}

/// A globe for the web, a window for desktop apps, with the apps' names.
fn place_line(p: Paint, task: &Task) -> Option<El> {
    let (glyph, label) = match task.place() {
        Place::Web => (&GLOBE, tr("On the web")),
        Place::Desktop => (&PANEL, tr("Desktop")),
        Place::Unknown => return None,
    };
    let mut apps: Vec<&str> = Vec::new();
    for window in &task.windows {
        if !apps.contains(&window.app.as_str()) {
            apps.push(&window.app);
        }
    }
    let words = if apps.is_empty() {
        label.to_owned()
    } else {
        apps.join(" · ")
    };
    Some(
        row()
            .key("place")
            .gap(5.0)
            .center_y()
            .label(label)
            .child(
                icon(glyph, 12.0)
                    .key("place-glyph")
                    .color(p.faint())
                    .a11y_hidden(),
            )
            .child(
                text(words)
                    .key("place-text")
                    .truncate()
                    .font(p.k.ty.small)
                    .color(p.faint())
                    .min_w(0.0),
            ),
    )
}

/// Its last few steps while it's live (the newest last); once it's over,
/// its outcome's first line.
fn steps(p: Paint, task: &Task) -> Vec<El> {
    let line = |key: String, words: String, color| {
        text(words)
            .key(key)
            .truncate()
            .font(p.k.ty.small)
            .color(color)
            .min_w(0.0)
    };
    match task.phase {
        Phase::Waiting { until_ms } => vec![line(
            "waiting".into(),
            tr_fill("Waiting for limit · resumes {}", &when::clock(until_ms)),
            p.muted(),
        )],
        Phase::Done | Phase::Failed | Phase::Stopped => task
            .detail()
            .map(|detail| {
                let color = if task.phase == Phase::Failed {
                    p.danger()
                } else {
                    p.muted()
                };
                line("outcome".into(), detail, color)
            })
            .into_iter()
            .collect(),
        Phase::Queued | Phase::Running | Phase::Paused => {
            let last = task.steps.len().saturating_sub(1);
            task.steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let color = if index == last { p.muted() } else { p.faint() };
                    line(format!("step-{index}"), step.clone(), color)
                })
                .collect()
        }
    }
}

/// Take over or Hand back, and ×, for a live task.
fn buttons<S: 'static>(p: Paint, task: &Task, actions: Actions<S>) -> Option<El> {
    if !task.phase.live() {
        return None;
    }
    let id = task.id.clone();
    let hand = match task.phase {
        Phase::Running => {
            let take_over = actions.take_over;
            let id = id.clone();
            Some(
                Button::new(tr("Take over"))
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut S| take_over(s, &id))
                    .build(p.k)
                    .key("take-over"),
            )
        }
        Phase::Paused => {
            let hand_back = actions.hand_back;
            let id = id.clone();
            Some(
                Button::new(tr("Hand back"))
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut S| hand_back(s, &id))
                    .build(p.k)
                    .key("hand-back"),
            )
        }
        _ => None,
    };
    let stop = actions.stop;
    Some(
        row()
            .key("buttons")
            .pt(6.0)
            .gap(6.0)
            .center_y()
            .children(hand)
            .child(
                Button::new(tr("Stop task"))
                    .variant(ButtonVariant::GhostMuted)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut S| stop(s, &id))
                    .build(p.k)
                    .key("stop"),
            ),
    )
}

/// Running: the activity spinner in the task's colour; otherwise a word,
/// a countdown, or a mark.
pub(crate) fn status(p: Paint, task: &Task, colour: rune::Color) -> El {
    match task.phase {
        Phase::Running => Spinner::new()
            .size(14.0)
            .color(colour)
            .build(p.k)
            .key("spinner"),
        Phase::Queued => tag(p, tr("Queued"), "queued"),
        Phase::Paused => tag(p, tr("You have it"), "paused"),
        Phase::Waiting { until_ms } => {
            let minutes = when::minutes_left(until_ms, when::now_ms()).max(1);
            tag(p, &tr_fill("{} min", &minutes.to_string()), "countdown")
        }
        Phase::Stopped => tag(p, tr("Stopped"), "stopped"),
        Phase::Done => icon(p.k.glyph.check, 16.0)
            .key("done")
            .color(p.success())
            .label(tr("Done")),
        Phase::Failed => icon(p.k.glyph.circle_x, 16.0)
            .key("failed")
            .color(p.danger())
            .label(tr("Failed")),
    }
}

fn tag(p: Paint, word: &str, key: &str) -> El {
    text(word)
        .key(key)
        .font(p.k.ty.small_medium)
        .color(p.faint())
}
