//! Background jobs at the orb, seen without hovering or opening Chats: a
//! calm card per running job (its colour, title, step, how long it runs)
//! with every browser it has live and small, a press on one opening it
//! large. More jobs than there's room for fold into one line with their
//! count; resting the pointer on it unfolds every job's full card below it
//! (scrolling past the panel's height), and leaving folds them back. A
//! press on the line keeps them open; its list button opens the task list.
//!
//! The orb's cluster has one order, nearest the orb first, so nothing
//! overlaps: the question waiting on the person, a task the person holds
//! or that waits out a limit, the job cards (or their line), then the pops
//! (done, failed). A question takes a card's room.

use std::time::Instant;

use rune::{El, col, el, icon, row, text};

use super::super::controls::Paint;
use super::super::icons::{GLOBE, PANEL};
use super::board::{Phase, Task};
use super::live::{self, Preview};
use super::{orb, palette, stack};
use crate::assistant::Origin;
use crate::desktop::i18n::{tr, tr_fill};

/// Job cards shown, with no question waiting; one fewer with one.
pub(crate) const MAX_CARDS: usize = 3;
/// Browsers shown on a card; more show as "+N".
pub(crate) const MAX_THUMBS: usize = 4;
/// A card's width, as the orb's bubbles.
pub(crate) const CARD_W: f32 = 320.0;
/// A browser's thumbnail on a card.
const THUMB_W: f32 = 70.0;
const THUMB_H: f32 = 44.0;
/// The count line's height.
pub(crate) const LINE_H: f32 = 30.0;
/// How long the pointer rests on the count line before every card unfolds,
/// and how long it stays away before they fold back.
pub(crate) const UNFOLD_AFTER: std::time::Duration = std::time::Duration::from_millis(60);
pub(crate) const FOLD_AFTER: std::time::Duration = std::time::Duration::from_millis(350);

/// What the count line asks of the host, beside the list's actions.
pub(crate) struct LineActions<S> {
    /// The pointer came onto the line or its unfolded cards (`true`), or
    /// left them.
    pub(crate) hover: fn(&mut S, bool),
    /// A press on the line: keep every card unfolded, or fold them again.
    pub(crate) pin: fn(&mut S),
}

impl<S> Clone for LineActions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for LineActions<S> {}

/// How the jobs show at the orb.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layout {
    /// No job runs.
    None,
    /// A card each, newest first (task ids).
    Cards(Vec<String>),
    /// Too many for cards: one line saying how many run.
    Line(usize),
    /// The line, unfolded: every job's card below it, newest first.
    All(Vec<String>),
}

/// The jobs at the orb: Background (or Chat) tasks queued or running.
/// One the person holds, or that waits out a limit, keeps its standing
/// bubble; one asking keeps its question.
pub(crate) fn running(tasks: &[Task]) -> Vec<&Task> {
    tasks
        .iter()
        .filter(|task| task.origin != Origin::ActionShortcut)
        .filter(|task| matches!(task.phase, Phase::Queued | Phase::Running))
        .collect()
}

/// How `jobs` (in the board's order, oldest first) show, with a question
/// waiting or not, and with the count line unfolded (`all`) or not.
pub(crate) fn layout(jobs: &[&Task], question: bool, all: bool) -> Layout {
    let room = if question { MAX_CARDS - 1 } else { MAX_CARDS };
    let newest_first = || jobs.iter().rev().map(|task| task.id.clone()).collect();
    match jobs.len() {
        0 => Layout::None,
        n if n > room && all => Layout::All(newest_first()),
        n if n > room => Layout::Line(n),
        _ => Layout::Cards(newest_first()),
    }
}

/// Of `count` browsers, how many show and how many more there are.
pub(crate) fn thumbs(count: usize) -> (usize, usize) {
    let shown = count.min(MAX_THUMBS);
    (shown, count - shown)
}

/// How long a job runs, for its card: whole minutes, at least one.
pub(crate) fn minutes(since: Option<Instant>, now: Instant) -> Option<u64> {
    since.map(|since| (now.saturating_duration_since(since).as_secs() / 60).max(1))
}

/// Where the jobs unfold and how tall the unfolded cards may get.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Room {
    /// Downward from the orb (else upward: the line sits at the bottom,
    /// nearest the orb).
    pub(crate) down: bool,
    /// The unfolded cards' greatest height; they scroll past it.
    pub(crate) max_h: f32,
}

/// The cards (or the line) for `jobs`, as laid out.
pub(crate) fn view<S: 'static>(
    p: Paint,
    jobs: &[&Task],
    layout: &Layout,
    room: Room,
    actions: stack::Actions<S>,
    line_actions: LineActions<S>,
) -> Vec<El> {
    let cards = |ids: &[String]| -> Vec<El> {
        ids.iter()
            .filter_map(|id| jobs.iter().find(|task| &task.id == id))
            .map(|task| card(p, task, &live::previews(&task.id), actions))
            .collect()
    };
    match layout {
        Layout::None => Vec::new(),
        Layout::Cards(ids) => cards(ids),
        Layout::Line(count) => vec![unfolding(
            p,
            line(p, jobs, *count, false, actions, line_actions),
            None,
            room,
            line_actions,
        )],
        Layout::All(ids) => {
            let mut cards = cards(ids);
            if !room.down {
                // Newest nearest the orb, which is below.
                cards.reverse();
            }
            let list = col()
                .key("jobs-all")
                .w(CARD_W)
                .max_h(room.max_h)
                .min_h(0.0)
                .shrink()
                .gap(8.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: !room.down,
                })
                .scroll_edge_fade(16.0)
                .role("list")
                .label(tr_fill("{} jobs running", &ids.len().to_string()))
                .children(cards);
            vec![unfolding(
                p,
                line(p, jobs, ids.len(), true, actions, line_actions),
                Some(list),
                room,
                line_actions,
            )]
        }
    }
}

/// The count line and, unfolded, every card past it, as one hover target:
/// moving from the line onto the cards keeps them open.
fn unfolding<S: 'static>(
    _p: Paint,
    line: El,
    cards: Option<El>,
    room: Room,
    line_actions: LineActions<S>,
) -> El {
    let hover = line_actions.hover;
    let mut parts: Vec<El> = vec![line];
    parts.extend(cards);
    if !room.down {
        parts.reverse();
    }
    col()
        .key("jobs-unfolding")
        .gap(8.0)
        .min_h(0.0)
        .shrink()
        .hover_delays(UNFOLD_AFTER, FOLD_AFTER)
        .on_hover_change(move |s: &mut S, inside, _| hover(s, inside))
        .animate_layout()
        .children(parts)
}

/// One job: dot, title and how long; its step; its browsers.
fn card<S: 'static>(
    p: Paint,
    task: &Task,
    previews: &[std::sync::Arc<Preview>],
    actions: stack::Actions<S>,
) -> El {
    let c = &p.k.c;
    let colour = palette::task(task.colour, p.k.dark);
    let open = actions.open;
    let id = task.id.clone();
    let mut head = row()
        .key("job-head")
        .w_full()
        .gap(7.0)
        .center_y()
        .child(orb::dot(task, colour, 8.0))
        .child(
            text(task.title.clone())
                .key("job-title")
                .truncate()
                .font(p.k.ty.small_medium)
                .color(p.text())
                .min_w(0.0)
                .grow(1.0),
        );
    if let Some(minutes) = minutes(task.since, Instant::now()) {
        head = head.child(
            text(tr_fill("{} min", &minutes.to_string()))
                .key("job-time")
                .font(p.k.ty.small)
                .color(p.faint()),
        );
    }
    let step = task.step.clone().map(|step| {
        text(step)
            .key("job-step")
            .truncate()
            .font(p.k.ty.small)
            .color(p.muted())
            .min_w(0.0)
    });
    let (shown, more) = thumbs(previews.len());
    let browsers = (shown > 0).then(|| {
        row()
            .key("job-browsers")
            .gap(6.0)
            .pt(2.0)
            .role("list")
            .label(tr("Live browsers"))
            .children(
                previews[..shown]
                    .iter()
                    .map(|preview| thumb(p, &task.id, preview, actions)),
            )
            .children((more > 0).then(|| {
                el().key("job-more")
                    .size(THUMB_H, THUMB_H)
                    .radius(6.0)
                    .fill(c.layer_02)
                    .center()
                    .child(
                        text(format!("+{more}"))
                            .font(p.k.ty.small_medium)
                            .color(p.muted()),
                    )
            }))
    });
    col()
        .key(format!("job-{}", task.id))
        .w(CARD_W)
        .padding(10.0)
        .gap(4.0)
        .radius(14.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 3.0,
            blur: 12.0,
            spread: 0.0,
            color: rune::Color::BLACK.alpha(if p.k.dark { 0.4 } else { 0.12 }),
            inset: false,
        })
        .role("group")
        .label(task.title.clone())
        .hit()
        .on_press(move |s: &mut S| open(s, &id))
        .child(head)
        .children(step)
        .children(browsers)
}

/// One browser on a card, sharp from its thumbnail stream; a press opens it
/// large in the overlay.
fn thumb<S: 'static>(p: Paint, task: &str, preview: &Preview, actions: stack::Actions<S>) -> El {
    let c = &p.k.c;
    let base = el()
        .key(format!("job-thumb-{}", preview.session))
        .size(THUMB_W, THUMB_H)
        .radius(6.0)
        .fill(c.layer_02)
        .border(p.hair, p.k.alpha(c.text_base, 0.14));
    let picture = if preview.has_frame() {
        base.pixels_fit(&preview.pixels, rune::SurfaceFit::Fill)
    } else {
        base.center()
            .child(icon(&GLOBE, 14.0).color(p.faint()).a11y_hidden())
    };
    let open = actions.open_preview;
    let (task, session) = (task.to_owned(), preview.session.clone());
    p.k.interactive(
        picture
            .role("button")
            .label(tr_fill(
                "Open {} live",
                if preview.name.is_empty() {
                    tr("Main browser")
                } else {
                    &preview.name
                },
            ))
            .hit()
            .on_press(move |s: &mut S| open(s, &task, &session)),
    )
}

/// More jobs than room: one line with each job's dot and how many run.
/// Resting on it unfolds every card (`open`); a press keeps them; the list
/// button opens the task list with Stop and Take over.
fn line<S: 'static>(
    p: Paint,
    jobs: &[&Task],
    count: usize,
    open: bool,
    actions: stack::Actions<S>,
    line_actions: LineActions<S>,
) -> El {
    let c = &p.k.c;
    let pin = line_actions.pin;
    let toggle = actions.toggle;
    let shown_dots = 6;
    let more_dots = count.saturating_sub(shown_dots);
    let label = tr_fill("{} jobs running", &count.to_string());
    row()
        .key("jobs-line")
        .h(LINE_H)
        .max_w(CARD_W)
        .pl(10.0)
        .pr(3.0)
        .gap(6.0)
        .center_y()
        .radius(LINE_H / 2.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .role("button")
        .label(if open {
            tr_fill("{}: hide the cards", &label)
        } else {
            tr_fill("{}: show every card", &label)
        })
        .hit()
        .on_press(move |s: &mut S| pin(s))
        .children(
            jobs.iter()
                .rev()
                .take(shown_dots)
                .map(|task| orb::dot(task, palette::task(task.colour, p.k.dark), 7.0)),
        )
        .children((more_dots > 0).then(|| {
            text(format!("+{more_dots}"))
                .key("jobs-line-more")
                .font(p.k.ty.small)
                .color(p.faint())
        }))
        .child(
            text(label)
                .key("jobs-line-text")
                .font(p.k.ty.small_medium)
                .color(p.text()),
        )
        .child(
            rune_kit::IconButton::new(&PANEL)
                .variant(rune_kit::IconButtonVariant::GhostMuted)
                .control_size(rune_kit::ControlSize::Small)
                .label(tr("Open the task list"))
                .on_press(move |s: &mut S| toggle(s))
                .build(p.k)
                .key("jobs-line-list"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::Event;

    fn board_with(jobs: usize, here: bool) -> super::super::board::Board {
        let mut board = super::super::board::Board::default();
        let now = Instant::now();
        for n in 0..jobs {
            board.apply(
                &Event::Started {
                    conversation: format!("job{n}"),
                    origin: Origin::Background,
                    question: format!("Job {n}"),
                },
                None,
                now,
            );
        }
        if here {
            board.apply(
                &Event::Started {
                    conversation: "here".into(),
                    origin: Origin::ActionShortcut,
                    question: "What's this?".into(),
                },
                None,
                now,
            );
        }
        board
    }

    #[test]
    fn jobs_get_a_card_each_newest_first_and_a_question_takes_one_s_room() {
        let board = board_with(3, true);
        let jobs = running(&board.tasks);
        assert_eq!(jobs.len(), 3, "a Here question is no job");
        assert_eq!(
            layout(&jobs, false, false),
            Layout::Cards(vec!["job2".into(), "job1".into(), "job0".into()])
        );
        assert_eq!(layout(&jobs, true, false), Layout::Line(3));
        assert_eq!(
            layout(&jobs[..2], true, false),
            Layout::Cards(vec!["job1".into(), "job0".into()])
        );
        assert_eq!(layout(&[], false, false), Layout::None);
    }

    #[test]
    fn many_jobs_fold_into_one_line() {
        let board = board_with(7, false);
        assert_eq!(
            layout(&running(&board.tasks), false, false),
            Layout::Line(7)
        );
    }

    #[test]
    fn resting_on_the_line_unfolds_every_job_newest_first() {
        let board = board_with(50, true);
        let jobs = running(&board.tasks);
        let Layout::All(ids) = layout(&jobs, true, true) else {
            panic!("the line unfolds into every card");
        };
        assert_eq!(ids.len(), 50, "every job, none summarised away");
        assert_eq!(ids.first().map(String::as_str), Some("job49"));
        assert_eq!(ids.last().map(String::as_str), Some("job0"));
        // Few enough jobs are cards anyway; unfolding changes nothing.
        assert_eq!(
            layout(&jobs[..2], false, true),
            Layout::Cards(vec!["job1".into(), "job0".into()])
        );
    }

    #[test]
    fn a_card_shows_four_browsers_and_counts_the_rest() {
        assert_eq!(thumbs(0), (0, 0));
        assert_eq!(thumbs(3), (3, 0));
        assert_eq!(thumbs(4), (4, 0));
        assert_eq!(thumbs(12), (4, 8));
    }

    #[test]
    fn a_job_runs_whole_minutes_and_at_least_one() {
        let start = Instant::now();
        assert_eq!(minutes(None, start), None);
        assert_eq!(minutes(Some(start), start), Some(1));
        assert_eq!(
            minutes(Some(start), start + std::time::Duration::from_secs(185)),
            Some(3)
        );
    }
}
