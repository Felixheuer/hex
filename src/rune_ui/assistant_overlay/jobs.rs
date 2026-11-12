//! Background jobs at the orb, seen without hovering or opening Chats: a
//! calm card per running job (its colour, title, step, how long it runs)
//! with every browser it has live and small, a press on one opening it
//! large. More jobs than there's room for fold into one line, and the list
//! (hover, or a press) shows them all.
//!
//! The orb's cluster has one order, nearest the orb first, so nothing
//! overlaps: the question waiting on the person, a task the person holds
//! or that waits out a limit, the job cards (or their line), then the pops
//! (done, failed). A question takes a card's room.

use std::time::Instant;

use rune::{El, col, el, icon, row, text};

use super::super::controls::Paint;
use super::super::icons::GLOBE;
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

/// How the jobs show at the orb.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layout {
    /// No job runs.
    None,
    /// A card each, newest first (task ids).
    Cards(Vec<String>),
    /// Too many for cards: one line saying how many run.
    Line(usize),
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
/// waiting or not.
pub(crate) fn layout(jobs: &[&Task], question: bool) -> Layout {
    let room = if question { MAX_CARDS - 1 } else { MAX_CARDS };
    match jobs.len() {
        0 => Layout::None,
        n if n > room => Layout::Line(n),
        _ => Layout::Cards(jobs.iter().rev().map(|task| task.id.clone()).collect()),
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

/// The cards (or the line) for `jobs`, as laid out.
pub(crate) fn view<S: 'static>(
    p: Paint,
    jobs: &[&Task],
    layout: &Layout,
    actions: stack::Actions<S>,
) -> Vec<El> {
    match layout {
        Layout::None => Vec::new(),
        Layout::Line(count) => vec![line(p, jobs, *count, actions)],
        Layout::Cards(ids) => ids
            .iter()
            .filter_map(|id| jobs.iter().find(|task| &task.id == id))
            .map(|task| card(p, task, &live::previews(&task.id), actions))
            .collect(),
    }
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
fn line<S: 'static>(p: Paint, jobs: &[&Task], count: usize, actions: stack::Actions<S>) -> El {
    let c = &p.k.c;
    let toggle = actions.toggle;
    row()
        .key("jobs-line")
        .h(30.0)
        .max_w(CARD_W)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(15.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .role("button")
        .label(tr_fill("{} jobs running", &count.to_string()))
        .hit()
        .on_press(move |s: &mut S| toggle(s))
        .children(
            jobs.iter()
                .rev()
                .take(8)
                .map(|task| orb::dot(task, palette::task(task.colour, p.k.dark), 7.0)),
        )
        .child(
            text(tr_fill("{} jobs running", &count.to_string()))
                .key("jobs-line-text")
                .font(p.k.ty.small_medium)
                .color(p.text()),
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
            layout(&jobs, false),
            Layout::Cards(vec!["job2".into(), "job1".into(), "job0".into()])
        );
        assert_eq!(layout(&jobs, true), Layout::Line(3));
        assert_eq!(
            layout(&jobs[..2], true),
            Layout::Cards(vec!["job1".into(), "job0".into()])
        );
        assert_eq!(layout(&[], false), Layout::None);
    }

    #[test]
    fn many_jobs_fold_into_one_line() {
        let board = board_with(7, false);
        assert_eq!(layout(&running(&board.tasks), false), Layout::Line(7));
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
