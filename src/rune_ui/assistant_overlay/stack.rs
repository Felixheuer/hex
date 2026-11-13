//! The task list: a sheet that unfolds from the orb (double-tap the Action
//! shortcut, press the orb, a job's row, or its "more" line) with every
//! Background task in three groups ([`Group`]): Waiting for you (a question
//! to answer, with Later), Running, and Done (the results the orb recalls,
//! [`super::results`]), empty ones left out; newest nearest the orb within
//! each. Each running row shows its colour, its question, where it works (a
//! globe for the web, a window for desktop apps, with their names), its
//! plan or last few steps, and its state: Queued, Running, "You have it"
//! (paused), or the minutes until a usage limit lifts. Take over and Hand
//! back pass a running task between the agent and the person; Stop task
//! stops one, Stop all every one (with two or more). A result offers what
//! it names to open, or Try again; a press brings its bubble back. While
//! the pointer rests on a row, its browsers stream live below it
//! ([`live`]); pressing one opens it large. The rows scroll; ×, Escape, or
//! a click outside the panel folds the list back.

use rune::{El, ScrollSpec, col, icon, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant};

use super::super::controls::Paint;
use super::super::icons::{GLOBE, PANEL};
use super::ask::ReplyActions;
use super::board::{Phase, Place, Task};
use super::live::{self, Preview};
use super::look::{self, Kind, State};
use super::results::{self, Outcome};
use super::{palette, when};
use crate::desktop::i18n::{tr, tr_fill};

/// The sheet's width.
pub(crate) const WIDTH: f32 = look::RAIL_W;
const ROW_GAP: f32 = 4.0;

/// What a press asks of the host.
pub(crate) struct Actions<S> {
    /// Stop task: stop it.
    pub(crate) stop: fn(&mut S, &str),
    /// Take over a running task: its desktop actions wait.
    pub(crate) take_over: fn(&mut S, &str),
    /// Hand a taken-over task back.
    pub(crate) hand_back: fn(&mut S, &str),
    /// Stop every task.
    pub(crate) stop_all: fn(&mut S),
    /// Fold the list back into the orb.
    pub(crate) toggle: fn(&mut S),
    /// Put a task's question bubble away (it still waits for the answer).
    pub(crate) dismiss: fn(&mut S, &str),
    /// The pointer came onto a row (`true`) or left it.
    pub(crate) hover: fn(&mut S, &str, bool),
    /// A live preview: open that browser large (task, session).
    pub(crate) open_preview: fn(&mut S, &str, &str),
    /// Later on a question: behind the other questions.
    pub(crate) defer: fn(&mut S, &str),
    /// A typed answer to a question.
    pub(crate) reply: ReplyActions<S>,
    /// A finished job's result.
    pub(crate) results: results::Actions<S>,
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

/// Where a task stands in the list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Group {
    /// It asks the person something.
    Waiting,
    /// Queued, running, taken over, or waiting out a usage limit.
    Running,
    /// Answered or failed: the results the orb recalls.
    Done,
}

impl Group {
    /// Top to bottom.
    pub(crate) const ALL: [Group; 3] = [Group::Waiting, Group::Running, Group::Done];

    pub(crate) fn of(task: &Task) -> Self {
        match task.phase {
            Phase::NeedsInput => Self::Waiting,
            Phase::Queued | Phase::Running | Phase::Paused | Phase::Waiting { .. } => Self::Running,
            Phase::Done | Phase::Failed | Phase::Stopped => Self::Done,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Waiting => tr("Waiting for you"),
            Self::Running => tr("Running"),
            Self::Done => tr("Done"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Done => "done",
        }
    }
}

/// The sheet with `tasks` (oldest first) and the `done` results (newest
/// first), at most `max_h` tall; its rows scroll past that. `newest_first`
/// lists the newest at the top (the sheet hangs below the rail's head).
/// The `hovered` task shows its browsers live.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sheet<S: 'static>(
    p: Paint,
    tasks: &[Task],
    done: &[&Outcome],
    newest_first: bool,
    max_h: f32,
    hovered: Option<&str>,
    actions: Actions<S>,
) -> El {
    let live = tasks.iter().filter(|task| task.phase.live()).count();
    let stop_all = (live >= 2).then(|| {
        let stop_all = actions.stop_all;
        look::button(p, Kind::Quiet, tr("Stop all"), move |s: &mut S| stop_all(s)).key("stop-all")
    });
    let toggle = actions.toggle;
    let header = row()
        .key("sheet-header")
        .w_full()
        .h(36.0)
        .pl(12.0)
        .pr(4.0)
        .gap(4.0)
        .center_y()
        .child(
            text(tr("Assistant tasks"))
                .key("sheet-title")
                .font(look::label(p))
                .color(p.muted())
                .grow(1.0)
                .min_w(0.0),
        )
        .children(stop_all)
        .child(
            IconButton::new(p.k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(tr("Close"))
                .on_press(move |s: &mut S| toggle(s))
                .build(p.k)
                .key("fold"),
        );
    let mut sections: Vec<El> = Vec::new();
    for group in Group::ALL {
        let mut members: Vec<&Task> = tasks
            .iter()
            .filter(|task| task.phase.live() && Group::of(task) == group)
            .collect();
        if newest_first {
            members.reverse();
        }
        let mut results: Vec<&Outcome> = if group == Group::Done {
            done.to_vec()
        } else {
            Vec::new()
        };
        if !newest_first {
            results.reverse();
        }
        if members.is_empty() && results.is_empty() {
            continue;
        }
        sections.push(
            text(group.title())
                .key(format!("group-{}", group.key()))
                .font(look::meta(p))
                .color(p.faint())
                .pl(12.0)
                .pt(if sections.is_empty() { 2.0 } else { 10.0 })
                .pb(2.0)
                .role("heading"),
        );
        sections.extend(members.into_iter().map(|task| {
            let previews = if hovered == Some(task.id.as_str()) {
                live::previews(&task.id)
            } else {
                Vec::new()
            };
            task_row(p, task, &previews, actions)
        }));
        sections.extend(
            results
                .into_iter()
                .map(|outcome| results::list_row(p, outcome, actions.results)),
        );
    }
    let rows = super::wheel::scroller::<S>(
        col(),
        super::wheel::Surface::Orb,
        "sheet-rows",
        super::wheel::Axis::Y,
    )
    .key("sheet-rows")
    .w_full()
    .min_h(0.0)
    .shrink()
    .gap(ROW_GAP)
    .scroll(ScrollSpec {
        stick_bottom: !newest_first,
    })
    .scroll_edge_fade(16.0)
    // The wheel answers anywhere over the rows, not only on a row.
    .hit()
    .role("list")
    .label(tr("Assistant tasks"))
    .children(sections);
    look::material(
        p,
        col()
            .key("sheet")
            .w(WIDTH)
            .max_h(max_h)
            .padding(4.0)
            .pb(8.0),
    )
    .hit()
    .on_key(move |s: &mut S, key: &rune::KeyEvent, _| {
        if key.key == rune::Key::Escape {
            toggle(s);
            true
        } else {
            false
        }
    })
    .animate_layout()
    .child(header)
    .child(rows)
}

/// One task in the list: its dot (with its plan's ring), title, and state
/// chip; where it works and what it does ("Safari · in background ·
/// choosing a time"); its plan's checklist or latest steps; its browsers
/// while the pointer rests on it; and its actions.
fn task_row<S: 'static>(
    p: Paint,
    task: &Task,
    previews: &[std::sync::Arc<Preview>],
    actions: Actions<S>,
) -> El {
    let colour = palette::task(task.colour, p.k.dark);
    let live = task.phase.live();
    let head = row()
        .key("head")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(super::orb::marker(p, task, colour, 9.0))
        .child(
            text(task.title.clone())
                .key("title")
                .truncate()
                .font(look::label(p))
                .color(if live { p.text() } else { p.muted() })
                .grow(1.0)
                .min_w(0.0),
        )
        .child(status(p, task));
    let place = place_line(p, task);
    // A plan's checklist stands for the latest steps; a question or a
    // usage limit still says so above it.
    let mut detail = steps(p, task);
    if live && !task.plan.is_empty() {
        if matches!(task.phase, Phase::Queued | Phase::Running | Phase::Paused) {
            detail.clear();
        }
        detail.extend(checklist(p, task, colour));
    }
    let lines = col()
        .key("lines")
        .w_full()
        .min_w(0.0)
        .gap(3.0)
        .pl(STEP_INSET)
        .children(place)
        .children(detail)
        .children(strip(p, &task.id, previews, actions));
    let (hover, hovered_id) = (actions.hover, task.id.clone());
    let body = col()
        .key(format!("task-{}", task.id))
        .w_full()
        .px(12.0)
        .py(10.0)
        .gap(4.0)
        .radius(12.0)
        .role("listitem")
        .label(task.title.clone())
        .hit()
        .on_hover_change(move |s: &mut S, inside, _| hover(s, &hovered_id, inside))
        .child(head)
        .child(lines)
        .children(buttons(p, task, actions));
    let body = if matches!(task.phase, Phase::Waiting { .. }) {
        body.opacity(0.6)
    } else {
        body
    };
    p.k.interactive(body)
}

/// How far a row's lines are set in: past its dot.
const STEP_INSET: f32 = 17.0;

/// A live preview's size in the list.
const PREVIEW_W: f32 = 148.0;
const PREVIEW_H: f32 = 92.0;

/// The task's browsers, live, two to a line; `None` while it has none.
fn strip<S: 'static>(
    p: Paint,
    task: &str,
    previews: &[std::sync::Arc<Preview>],
    actions: Actions<S>,
) -> Option<El> {
    if previews.is_empty() {
        return None;
    }
    Some(
        row()
            .key("previews")
            .pt(6.0)
            .gap(6.0)
            .flex_wrap()
            .role("list")
            .label(tr("Live browsers"))
            .children(
                previews
                    .iter()
                    .map(|preview| thumbnail(p, task, preview, actions)),
            ),
    )
}

/// One browser: its live picture (or, for a light browser that draws none,
/// its page's title and address) and its name; a press opens it large.
fn thumbnail<S: 'static>(p: Paint, task: &str, preview: &Preview, actions: Actions<S>) -> El {
    let c = &p.k.c;
    let page = preview.page();
    let picture = if preview.has_frame() {
        rune::el()
            .key("picture")
            .size(PREVIEW_W, PREVIEW_H)
            .radius(6.0)
            .fill(c.layer_02)
            .pixels_fit(&preview.pixels, rune::SurfaceFit::Fill)
    } else {
        let words = if page.failed.is_some() && page.title.is_empty() && page.url.is_empty() {
            tr("No live picture").to_owned()
        } else if !page.title.is_empty() {
            page.title.clone()
        } else if !page.url.is_empty() {
            page.url.clone()
        } else {
            tr("Loading…").to_owned()
        };
        col()
            .key("card")
            .size(PREVIEW_W, PREVIEW_H)
            .radius(6.0)
            .fill(c.layer_02)
            .padding(8.0)
            .gap(4.0)
            .child(icon(&GLOBE, 14.0).color(p.faint()).a11y_hidden())
            .child(
                text(words)
                    .key("card-text")
                    .font(p.k.ty.small)
                    .color(p.muted())
                    .wrap()
                    .max_lines(3),
            )
    };
    let name = if preview.name.is_empty() {
        tr("Main browser").to_owned()
    } else {
        preview.name.clone()
    };
    let caption = match live::host_of(&page.url) {
        Some(host) => format!("{name} · {host}"),
        None => name,
    };
    let open = actions.open_preview;
    let (task, session) = (task.to_owned(), preview.session.clone());
    let card = col()
        .key(format!("preview-{}", preview.session))
        .w(PREVIEW_W)
        .gap(3.0)
        .role("button")
        .label(tr_fill("Open {} live", &caption))
        .hit()
        .on_press(move |s: &mut S| open(s, &task, &session))
        .child(picture.border(p.hair, p.k.alpha(c.text_base, 0.14)))
        .child(
            text(caption)
                .key("caption")
                .truncate()
                .font(p.k.ty.small)
                .color(p.faint())
                .min_w(0.0),
        );
    p.k.interactive(card)
}

/// A globe for the web, a window for desktop apps, with the app's name,
/// and "in background" while it runs there (the chip says the rest).
fn place_line(p: Paint, task: &Task) -> Option<El> {
    let app = super::jobs::app_of(task)?;
    let words = if task.phase == Phase::Running {
        super::jobs::whereabouts(task).unwrap_or(app)
    } else {
        app
    };
    let glyph = match task.place() {
        Place::Web => Some(&GLOBE),
        Place::Desktop => Some(&PANEL),
        Place::Unknown => None,
    };
    Some(
        row()
            .key("place")
            .gap(5.0)
            .center_y()
            .label(words.clone())
            .children(glyph.map(|glyph| {
                icon(glyph, 12.0)
                    .key("place-glyph")
                    .color(p.faint())
                    .a11y_hidden()
            }))
            .child(
                text(words)
                    .key("place-text")
                    .truncate()
                    .font(look::meta(p))
                    .color(p.muted())
                    .min_w(0.0),
            ),
    )
}

/// How many plan steps the list shows around the current one.
const PLAN_ROWS: usize = 6;

/// Its plan as a checklist, the step it's on highlighted in its colour,
/// with a window of [`PLAN_ROWS`] around it when the plan is long.
fn checklist(p: Paint, task: &Task, colour: rune::Color) -> Vec<El> {
    let current = task.current_step();
    let len = task.plan.len();
    let start = current
        .unwrap_or(len)
        .saturating_sub(2)
        .min(len.saturating_sub(PLAN_ROWS));
    let end = (start + PLAN_ROWS).min(len);
    let mut rows: Vec<El> = Vec::new();
    if start > 0 {
        rows.push(
            text(tr_fill("{} done", &start.to_string()))
                .key("plan-earlier")
                .font(p.k.ty.small)
                .color(p.faint()),
        );
    }
    for (index, step) in task.plan.iter().enumerate().take(end).skip(start) {
        let now = current == Some(index) && task.phase.live();
        let (mark, color, font): (El, _, _) = if step.done {
            (
                icon(p.k.glyph.check_small, 12.0).color(p.faint()),
                p.faint(),
                p.k.ty.small,
            )
        } else if now {
            (
                rune::el()
                    .size(7.0, 7.0)
                    .radius(3.5)
                    .fill(colour)
                    .ml(2.5)
                    .mr(2.5),
                p.text(),
                p.k.ty.small_medium,
            )
        } else {
            (
                rune::el()
                    .size(7.0, 7.0)
                    .radius(3.5)
                    .border(1.0, p.faint())
                    .ml(2.5)
                    .mr(2.5),
                p.muted(),
                p.k.ty.small,
            )
        };
        let row_el = row()
            .key(format!("plan-{index}"))
            .gap(6.0)
            .center_y()
            .child(mark.key("plan-mark").a11y_hidden())
            .child(
                text(step.title.clone())
                    .key("plan-text")
                    .truncate()
                    .font(font)
                    .color(color)
                    .min_w(0.0),
            );
        rows.push(if now {
            row_el
                .key("plan-current")
                .px(4.0)
                .ml(-4.0)
                .radius(6.0)
                .fill(colour.alpha(0.14))
        } else {
            row_el
        });
    }
    if end < len {
        rows.push(
            text(tr_fill("+{} more", &(len - end).to_string()))
                .key("plan-later")
                .font(p.k.ty.small)
                .color(p.faint()),
        );
    }
    rows
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
        Phase::NeedsInput => task
            .question
            .clone()
            .map(|question| line("asked".into(), question, p.text()))
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

/// A live task's actions, right-aligned under it: Take over (or Hand back,
/// the primary, while the person has it; Later on a question) and a quiet
/// Stop.
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
                look::button(p, Kind::Secondary, tr("Take over"), move |s: &mut S| {
                    take_over(s, &id)
                })
                .key("take-over"),
            )
        }
        Phase::Paused => {
            let hand_back = actions.hand_back;
            let id = id.clone();
            Some(
                look::button(p, Kind::Primary, tr("Hand back"), move |s: &mut S| {
                    hand_back(s, &id)
                })
                .key("hand-back"),
            )
        }
        Phase::NeedsInput => {
            let defer = actions.defer;
            let id = id.clone();
            Some(
                look::button(p, Kind::Secondary, tr("Later"), move |s: &mut S| {
                    defer(s, &id)
                })
                .key("later"),
            )
        }
        _ => None,
    };
    let stop = actions.stop;
    Some(
        row()
            .key("buttons")
            .w_full()
            .pt(6.0)
            .gap(8.0)
            .justify_end()
            .center_y()
            .child(
                look::button(p, Kind::Quiet, tr("Stop task"), move |s: &mut S| {
                    stop(s, &id)
                })
                .key("stop"),
            )
            .children(hand),
    )
}

/// A task's state chip (`look::State`, a glyph and a word): Working while
/// it runs, with the word saying more when it waits (Queued, You have it,
/// the minutes left of a usage limit); Needs you, Done, Failed; Stopped
/// as a plain word.
pub(crate) fn status(p: Paint, task: &Task) -> El {
    match task.phase {
        Phase::Running => look::chip(p, State::Working),
        Phase::Queued => look::chip_words(p, State::Working, tr("Queued")).key("queued"),
        Phase::Paused => look::chip_words(p, State::Working, tr("You have it")).key("paused"),
        Phase::NeedsInput => look::chip(p, State::NeedsYou).key("needs-input"),
        Phase::Waiting { until_ms } => {
            let minutes = when::minutes_left(until_ms, when::now_ms()).max(1);
            look::chip_words(p, State::Working, &tr_fill("{} min", &minutes.to_string()))
                .key("countdown")
        }
        Phase::Stopped => text(tr("Stopped"))
            .key("stopped")
            .font(look::label(p))
            .color(p.faint()),
        Phase::Done => look::chip(p, State::Done).key("done"),
        Phase::Failed => look::chip(p, State::Failed).key("failed"),
    }
}
