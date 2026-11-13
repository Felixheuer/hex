//! Background jobs at the orb, seen without opening anything: a calm,
//! compact row per running job (its colour and plan ring, title, how long
//! it runs, the step it's on, live as it changes) with every browser and
//! app window it works in live and small, a press on one opening it large.
//! At most [`MAX_ROWS`] rows show (one fewer while a question waits), the
//! newest first, in the order the jobs started, so nothing jumps; the rest
//! fold into one line ("3 more tasks") whose press opens the task list,
//! where every job is, scrolling. "All N live" on a row opens the live wall
//! with every picture of the job and its helpers.
//!
//! The orb's cluster has one order, nearest the orb first, so nothing
//! overlaps: the question waiting on the person, a task the person holds
//! or that waits out a limit, the job rows (and their line), then the
//! results ([`super::results`]).

use std::time::Instant;

use rune::{El, col, el, icon, row, text};

use super::super::controls::Paint;
use super::super::icons::{GLOBE, PANEL};
use super::app_live::{self, AppPreview};
use super::board::{Phase, Task};
use super::live::{self, Preview};
use super::{orb, palette, stack};
use crate::assistant::Origin;
use crate::desktop::i18n::{tr, tr_fill};

/// Job rows shown, with no question waiting; one fewer with one.
pub(crate) const MAX_ROWS: usize = 3;
/// Browsers shown on a row; more show as "+N".
pub(crate) const MAX_THUMBS: usize = 4;
/// A row's width, as the orb's bubbles.
pub(crate) const CARD_W: f32 = 320.0;
/// A browser's thumbnail on a row.
const THUMB_W: f32 = 70.0;
const THUMB_H: f32 = 44.0;
/// The "more" line's height.
pub(crate) const LINE_H: f32 = 30.0;

/// What a job row asks of the host, beside the list's actions.
pub(crate) struct LineActions<S> {
    /// "All N live" on a job's row: every browser and window of it and its
    /// helpers on the live wall (the job's id).
    pub(crate) wall: fn(&mut S, &str),
}

impl<S> Clone for LineActions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for LineActions<S> {}

/// How the jobs show at the orb.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layout {
    /// A row each, newest first (task ids).
    pub(crate) rows: Vec<String>,
    /// Jobs (leads) beyond the rows, summed up in one line.
    pub(crate) more: usize,
}

/// The jobs at the orb: Background (or Chat) tasks queued or running.
/// One the person holds, or that waits out a limit, keeps its standing
/// bubble; one asking keeps its question.
pub(crate) fn running(tasks: &[Task]) -> Vec<&Task> {
    tasks
        .iter()
        // Jobs: a Chats request that became one was escalated (Background
        // here); a plain Chats reply gets no row.
        .filter(|task| task.origin == Origin::Background)
        .filter(|task| matches!(task.phase, Phase::Queued | Phase::Running))
        .collect()
}

/// How `jobs` (in the board's order, oldest first) show, with a question
/// waiting or not.
///
/// A job's helpers (`assistant::agent::team`) fold into their lead's row
/// (and count in its "N helpers"); the list shows each.
pub(crate) fn layout(jobs: &[&Task], question: bool) -> Layout {
    let room = if question { MAX_ROWS - 1 } else { MAX_ROWS };
    let led = |task: &Task| {
        task.lead
            .as_ref()
            .is_some_and(|lead| jobs.iter().any(|job| &job.id == lead))
    };
    let leads: Vec<&Task> = jobs
        .iter()
        .rev()
        .copied()
        .filter(|task| !led(task))
        .collect();
    let rows: Vec<String> = leads
        .iter()
        .take(room)
        .map(|task| task.id.clone())
        .collect();
    Layout {
        more: leads.len() - rows.len(),
        rows,
    }
}
/// A lead's helpers as its card sums them up: how many, how many ended.
fn helpers_of<'a>(team: &[&'a Task], lead: &str) -> Vec<&'a Task> {
    team.iter()
        .copied()
        .filter(|task| task.lead.as_deref() == Some(lead))
        .collect()
}

/// A card's one strip of live pictures: of `apps` app windows and
/// `browsers` browsers, how many of each show (apps first, the windows it
/// works in on the desktop) and how many more there are, at most
/// [`MAX_THUMBS`] in all.
pub(crate) fn strip(apps: usize, browsers: usize) -> (usize, usize, usize) {
    let apps_shown = apps.min(MAX_THUMBS);
    let browsers_shown = browsers.min(MAX_THUMBS - apps_shown);
    (
        apps_shown,
        browsers_shown,
        apps + browsers - apps_shown - browsers_shown,
    )
}

/// How long a job runs, for its card: whole minutes, at least one.
pub(crate) fn minutes(since: Option<Instant>, now: Instant) -> Option<u64> {
    since.map(|since| (now.saturating_duration_since(since).as_secs() / 60).max(1))
}

/// `minutes` as a card says it: "12 min", "2 h 5 min", "3 d 4 h" (jobs run
/// up to seven days).
pub(crate) fn running_for(minutes: u64) -> String {
    let (days, hours, rest) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    let h = |hours: u64| tr_fill("{} h", &hours.to_string());
    let min = |minutes: u64| tr_fill("{} min", &minutes.to_string());
    match (days, hours, rest) {
        (0, 0, rest) => min(rest),
        (0, hours, 0) => h(hours),
        (0, hours, rest) => format!("{} {}", h(hours), min(rest)),
        (days, 0, _) => tr_fill("{} d", &days.to_string()),
        (days, hours, _) => format!("{} {}", tr_fill("{} d", &days.to_string()), h(hours)),
    }
}

/// Where the job rows sit and how tall they may get.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Room {
    /// Downward from the orb (else upward: the newest row sits at the
    /// bottom, nearest the orb).
    pub(crate) down: bool,
    /// The rows' greatest height; they scroll past it.
    pub(crate) max_h: f32,
    /// How a row comes and goes.
    pub(crate) appear: rune::Transition,
}

/// The rows (and the "more" line) for `jobs`, as laid out: one column that
/// takes the pointer, so the wheel and the gaps between rows answer, and
/// scrolls should it outgrow its room. `team` is every helper on the board
/// (any phase), for its lead's row.
pub(crate) fn view<S: 'static>(
    p: Paint,
    jobs: &[&Task],
    team: &[&Task],
    layout: &Layout,
    room: Room,
    actions: stack::Actions<S>,
    line_actions: LineActions<S>,
) -> Option<El> {
    if layout.rows.is_empty() {
        return None;
    }
    let mut rows: Vec<El> = layout
        .rows
        .iter()
        .filter_map(|id| jobs.iter().find(|task| &task.id == id))
        .map(|task| {
            card(
                p,
                task,
                &app_live::previews(&task.id),
                &live::previews(&task.id),
                &helpers_of(team, &task.id),
                actions,
                line_actions.wall,
            )
            .transition(room.appear)
        })
        .collect();
    if layout.more > 0 {
        let shown: Vec<&str> = layout.rows.iter().map(String::as_str).collect();
        let led = |task: &Task| {
            task.lead
                .as_ref()
                .is_some_and(|lead| jobs.iter().any(|job| &job.id == lead))
        };
        let rest: Vec<&Task> = jobs
            .iter()
            .rev()
            .copied()
            .filter(|task| !shown.contains(&task.id.as_str()) && !led(task))
            .collect();
        rows.push(line(p, &rest, layout.more, actions).transition(room.appear));
    }
    if !room.down {
        rows.reverse();
    }
    Some(
        col()
            .key("jobs")
            .w(CARD_W)
            .max_h(room.max_h)
            .min_h(0.0)
            .shrink()
            .gap(6.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: !room.down,
            })
            .scroll_edge_fade(16.0)
            .hit()
            .role("list")
            .label(tr_fill(
                "{} jobs running",
                &(layout.rows.len() + layout.more).to_string(),
            ))
            .animate_layout()
            .children(rows),
    )
}

/// One job: its dot (with its plan's ring), title, "All N live", and how
/// long it runs; the step it's on; its browsers and windows, live; its
/// helpers. A press opens the task list with its plan, Take over, and Stop.
fn card<S: 'static>(
    p: Paint,
    task: &Task,
    apps: &[std::sync::Arc<AppPreview>],
    previews: &[std::sync::Arc<Preview>],
    helpers: &[&Task],
    actions: stack::Actions<S>,
    open_wall: fn(&mut S, &str),
) -> El {
    let c = &p.k.c;
    let colour = palette::task(task.colour, p.k.dark);
    let toggle = actions.toggle;
    let (apps_shown, shown, more) = strip(apps.len(), previews.len());
    // Every picture of the job and its helpers, when the row can't show
    // them all: "All 20 live" opens the wall.
    let all = apps.len()
        + previews.len()
        + helpers
            .iter()
            .map(|helper| live::previews(&helper.id).len() + app_live::previews(&helper.id).len())
            .sum::<usize>();
    let wall = wants_wall(all, apps_shown + shown).then(|| {
        let id = task.id.clone();
        let label = tr_fill("All {} live", &all.to_string());
        p.k.interactive(
            el().key("job-wall")
                .h(20.0)
                .px(8.0)
                .radius(10.0)
                .fill(c.layer_02)
                .center()
                .role("button")
                .label(label.clone())
                .hit()
                .on_press(move |s: &mut S| open_wall(s, &id))
                .child(text(label).font(p.k.ty.small_medium).color(p.text())),
        )
    });
    let when = if task.phase == Phase::Queued {
        // Not only the faint dot: waiting is said.
        Some(
            text(tr("Queued"))
                .key("job-queued")
                .font(p.k.ty.small)
                .color(p.faint()),
        )
    } else {
        minutes(task.since, Instant::now()).map(|minutes| {
            text(running_for(minutes))
                .key("job-time")
                .font(p.k.ty.small)
                .color(p.faint())
        })
    };
    let head = row()
        .key("job-head")
        .w_full()
        .gap(7.0)
        .center_y()
        .child(orb::marker(p, task, colour, 8.0))
        .child(
            text(task.title.clone())
                .key("job-title")
                .truncate()
                .font(p.k.ty.small_medium)
                .color(p.text())
                .min_w(0.0)
                .grow(1.0),
        )
        .children(wall)
        .children(when);
    let step = card_step(task)
        .or_else(|| (task.phase == Phase::Running).then(|| tr("Thinking…").to_owned()))
        .map(|step| {
            text(step)
                .key("job-step")
                .truncate()
                .font(p.k.ty.small)
                .color(p.muted())
                .min_w(0.0)
                .pl(STEP_INSET)
        });
    let browsers = (apps_shown + shown > 0).then(|| {
        row()
            .key("job-browsers")
            .gap(6.0)
            .pt(2.0)
            .pl(STEP_INSET)
            .role("list")
            .label(tr("Live windows"))
            .children(
                apps[..apps_shown]
                    .iter()
                    .map(|preview| app_thumb(p, &task.id, preview, actions)),
            )
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
    let team = (!helpers.is_empty()).then(|| team_row(p, helpers).pl(STEP_INSET));
    let card = col()
        .key(format!("job-{}", task.id))
        .w(CARD_W)
        .px(10.0)
        .py(8.0)
        .gap(3.0)
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
        .role("button")
        .label(card_label(task))
        .hit()
        .on_press(move |s: &mut S| toggle(s))
        .child(head)
        .children(step)
        .children(browsers)
        .children(team);
    p.k.interactive(card)
}

/// How far a row's step, pictures, and helpers are set in: past its dot.
const STEP_INSET: f32 = 15.0;
/// Whether a card offers the live wall: `all` pictures of the job and its
/// helpers, of which it shows `shown` itself; two or more it can't all
/// show.
pub(crate) fn wants_wall(all: usize, shown: usize) -> bool {
    all >= 2 && all > shown
}

/// A lead's helpers on its card: a dot each and how many ended ("4 Helfer
/// · 1 erledigt"); resting on the cards shows each one's own.
fn team_row(p: Paint, helpers: &[&Task]) -> El {
    let ended = helpers
        .iter()
        .filter(|task| matches!(task.phase, Phase::Done | Phase::Failed | Phase::Stopped))
        .count();
    let mut words = tr_fill("{} helpers", &helpers.len().to_string());
    if ended > 0 {
        words = format!("{words} · {}", tr_fill("{} done", &ended.to_string()));
    }
    row()
        .key("job-team")
        .gap(5.0)
        .pt(2.0)
        .center_y()
        .role("group")
        .label(words.clone())
        .children(
            helpers
                .iter()
                .take(10)
                .map(|task| orb::dot(task, palette::task(task.colour, p.k.dark), 6.0)),
        )
        .child(
            text(words)
                .key("job-team-text")
                .font(p.k.ty.small)
                .color(p.muted()),
        )
}

/// A card's second line: the plan step it's on, as the job wrote its plan
/// (in the person's language), with how far it is ("3/7 · Preise
/// vergleichen"); before it has a plan, its latest action.
pub(crate) fn card_step(task: &Task) -> Option<String> {
    if let (Some(at), Some((done, all))) = (task.current_step(), task.progress()) {
        let title = task.plan.get(at).map(|step| step.title.trim())?;
        return Some(format!("{}/{all} · {title}", (done + 1).min(all)));
    }
    task.step.clone()
}

/// What a screen reader says for a card: its title, whether it waits, and
/// its step.
fn card_label(task: &Task) -> String {
    let mut label = task.title.clone();
    if task.phase == Phase::Queued {
        label = format!("{label}, {}", tr("Queued"));
    }
    if let Some(step) = card_step(task) {
        label = format!("{label}: {step}");
    }
    label
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

/// One app window a job works in, live from its window (even parked off
/// the screen), with the app's name; a press opens it large in the overlay.
fn app_thumb<S: 'static>(
    p: Paint,
    task: &str,
    preview: &AppPreview,
    actions: stack::Actions<S>,
) -> El {
    let c = &p.k.c;
    let base = el()
        .key(format!("job-app-{}", preview.number))
        .size(THUMB_W, THUMB_H)
        .radius(6.0)
        .fill(c.layer_02)
        .border(p.hair, p.k.alpha(c.text_base, 0.14));
    let picture = if preview.has_frame() {
        base.pixels_fit(&preview.pixels, rune::SurfaceFit::Fill)
    } else {
        // Its name until the first picture comes.
        base.center().px(4.0).child(
            text(preview.window.app.clone())
                .truncate()
                .font(p.k.ty.small)
                .color(p.faint()),
        )
    };
    let open = actions.open_preview;
    let (task, session) = (task.to_owned(), preview.session());
    let name = preview.window.app.clone();
    let captioned = col()
        .key(format!("job-app-cell-{}", preview.number))
        .w(THUMB_W)
        .gap(2.0)
        .child(picture)
        .child(
            text(name.clone())
                .key("job-app-name")
                .truncate()
                .font(p.k.ty.small)
                .color(p.muted())
                .min_w(0.0),
        );
    p.k.interactive(
        captioned
            .role("button")
            .label(tr_fill("Open {} live", &name))
            .hit()
            .on_press(move |s: &mut S| open(s, &task, &session)),
    )
}

/// The jobs past the rows: one line with each one's dot and how many; a
/// press (or its list button) opens the task list, where every job is.
fn line<S: 'static>(p: Paint, rest: &[&Task], count: usize, actions: stack::Actions<S>) -> El {
    let c = &p.k.c;
    let toggle = actions.toggle;
    let shown_dots = 6;
    let more_dots = rest.len().saturating_sub(shown_dots);
    let label = tr_fill("{} more tasks", &count.to_string());
    row()
        .key("jobs-line")
        .w(CARD_W)
        .h(LINE_H)
        .pl(12.0)
        .pr(3.0)
        .gap(6.0)
        .center_y()
        .radius(LINE_H / 2.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .role("button")
        .label(format!("{label}: {}", tr("Open the task list")))
        .hit()
        .on_press(move |s: &mut S| toggle(s))
        .children(
            rest.iter()
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
                .color(p.text())
                .grow(1.0)
                .min_w(0.0),
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

    fn rows(ids: &[&str], more: usize) -> Layout {
        Layout {
            rows: ids.iter().map(|id| (*id).to_owned()).collect(),
            more,
        }
    }

    #[test]
    fn jobs_get_a_row_each_newest_first_and_a_question_takes_one_s_room() {
        let board = board_with(3, true);
        let jobs = running(&board.tasks);
        assert_eq!(jobs.len(), 3, "a Here question is no job");
        assert_eq!(layout(&jobs, false), rows(&["job2", "job1", "job0"], 0));
        assert_eq!(layout(&jobs, true), rows(&["job2", "job1"], 1));
        assert_eq!(layout(&jobs[..2], true), rows(&["job1", "job0"], 0));
        assert_eq!(layout(&[], false), Layout::default());
    }

    #[test]
    fn many_jobs_keep_three_rows_and_count_the_rest() {
        let board = board_with(50, false);
        let jobs = running(&board.tasks);
        assert_eq!(
            layout(&jobs, false),
            rows(&["job49", "job48", "job47"], 47),
            "the newest three, none jumping as others work"
        );
        // A step on an older job doesn't move it into the rows.
        let mut board = board;
        board.apply(
            &Event::Step {
                conversation: "job3".into(),
                description: "Busy".into(),
            },
            None,
            Instant::now(),
        );
        assert_eq!(layout(&running(&board.tasks), false).rows[0], "job49");
    }

    #[test]
    fn helpers_fold_into_their_lead_s_row() {
        let mut board = board_with(4, false);
        for task in &mut board.tasks {
            if task.id != "job0" {
                task.lead = Some("job0".into());
            }
        }
        assert_eq!(layout(&running(&board.tasks), false), rows(&["job0"], 0));
    }

    #[test]
    fn a_card_shows_four_pictures_apps_first_and_counts_the_rest() {
        // Browsers alone, as before.
        assert_eq!(strip(0, 0), (0, 0, 0));
        assert_eq!(strip(0, 3), (0, 3, 0));
        assert_eq!(strip(0, 12), (0, 4, 8));
        // App windows first, browsers in the room left, one strip of four.
        assert_eq!(strip(1, 2), (1, 2, 0));
        assert_eq!(strip(2, 5), (2, 2, 3));
        assert_eq!(strip(6, 1), (4, 0, 3));
    }

    #[test]
    fn a_card_offers_the_wall_when_it_can_t_show_every_picture() {
        // Twenty helpers with a browser each, the lead's card shows none.
        assert!(wants_wall(20, 0));
        // Twelve browsers, four on the card.
        assert!(wants_wall(12, 4));
        // All on the card already, or a single one: no wall.
        assert!(!wants_wall(3, 3));
        assert!(!wants_wall(1, 0));
        assert!(!wants_wall(0, 0));
    }

    #[test]
    fn a_card_says_the_plan_step_it_s_on_in_the_plan_s_words() {
        use crate::assistant::types::PlanStep;
        let board = board_with(1, false);
        let mut task = board.tasks[0].clone();
        task.step = Some("Clicking “Accept” in Chrome".into());
        assert_eq!(
            card_step(&task).as_deref(),
            Some("Clicking “Accept” in Chrome")
        );
        task.plan = vec![
            PlanStep {
                title: "Lieferanten finden".into(),
                done: true,
            },
            PlanStep {
                title: "Preise vergleichen".into(),
                done: false,
            },
            PlanStep {
                title: "Tabelle schicken".into(),
                done: false,
            },
        ];
        assert_eq!(
            card_step(&task).as_deref(),
            Some("2/3 · Preise vergleichen")
        );
    }

    #[test]
    fn a_long_job_says_hours_and_days() {
        assert_eq!(running_for(12), "12 min");
        assert_eq!(running_for(60), "1 h");
        assert_eq!(running_for(125), "2 h 5 min");
        assert_eq!(running_for(3 * 1440), "3 d");
        assert_eq!(running_for(4320 + 4 * 60 + 7), "3 d 4 h");
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
