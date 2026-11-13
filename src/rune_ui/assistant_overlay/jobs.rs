//! Background jobs in the rail, seen without opening anything: a calm row
//! per running job in the card material (its colour and plan ring, title,
//! how long it runs; one line of where it works and what it does now,
//! "Safari · in background · choosing a time", live as it changes; every
//! browser and app window it works in, live and small, a press on one
//! opening it large; a thin plan bar). At most [`MAX_ROWS`] rows show, the
//! newest first, in the order the jobs started, so nothing jumps; the rest
//! fold into one "N more" line whose press opens the task list in place,
//! where every job is, scrolling. "All N live" on a row opens the live wall
//! with every picture of the job and its helpers.
//!
//! The rail has one order, nearest its head first, so nothing overlaps:
//! the question waiting on the person, a task the person holds or that
//! waits out a limit, the job rows (and their line), then the result
//! ([`super::results`]).

use std::time::Instant;

use rune::{El, col, el, row, text};

use super::super::controls::Paint;
use super::super::icons::{GLOBE, PANEL};
use super::app_live::{self, AppPreview};
use super::board::{Phase, Task};
use super::live::{self, Preview};
use super::{look, orb, palette, stack};
use crate::assistant::Origin;
use crate::desktop::i18n::{tr, tr_fill};

/// Job rows shown at most (`docs/ux/redesign.md` §3.6: the rail holds its
/// head, one question, three job rows, and one result).
pub(crate) const MAX_ROWS: usize = 3;
/// Browsers shown on a row; more show as "+N".
pub(crate) const MAX_THUMBS: usize = 2;
/// A row's width: the rail's.
pub(crate) const CARD_W: f32 = look::RAIL_W;
/// A mini on a row: two to a row, large enough for its motion to read.
pub(crate) const THUMB_W: f32 = 150.0;
pub(crate) const THUMB_H: f32 = 94.0;
/// The "more" line's height.
pub(crate) const LINE_H: f32 = 36.0;

/// What a job row asks of the host, beside the list's actions.
pub(crate) struct LineActions<S> {
    /// "All N live" on a job's row: every browser and window of it and its
    /// helpers on the live wall (the job's id).
    pub(crate) wall: fn(&mut S, &str),
    /// "All live" beside the orb: every live picture of every job and of
    /// the desk on the wall.
    pub(crate) all: fn(&mut S),
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

/// How `jobs` (in the board's order, oldest first) show: at most
/// [`MAX_ROWS`] rows, one fewer while a question waits on top (so the rail
/// stays within head + question + three rows + result on a short display).
///
/// A job's helpers (`assistant::agent::team`) fold into their lead's row
/// (and count in its "N helpers"); the list shows each.
pub(crate) fn layout(jobs: &[&Task], question: bool) -> Layout {
    layout_at_most(jobs, question, usize::MAX)
}

/// [`layout`] with at most `most` rows: the rail's share of the floating
/// cards (`docs/ux/redesign.md` §3.6, five on the screen at once) once
/// the island's card, the stage, the question, the held tasks, and the
/// result have theirs; the rest go to the "N more" line.
pub(crate) fn layout_at_most(jobs: &[&Task], question: bool, most: usize) -> Layout {
    let room = if question { MAX_ROWS - 1 } else { MAX_ROWS }.min(most);
    let led = |task: &Task| {
        task.lead
            .as_ref()
            .is_some_and(|lead| jobs.iter().any(|job| &job.id == lead))
    };
    let mut leads: Vec<&Task> = jobs
        .iter()
        .rev()
        .copied()
        .filter(|task| !led(task))
        .collect();
    // Working jobs before queued ones (newest first within each): with
    // few rows to give, the ones doing something get them.
    leads.sort_by_key(|task| task.phase == Phase::Queued);
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
#[derive(Clone, Debug)]
pub(crate) struct Room {
    /// Downward from the orb (else upward: the newest row sits at the
    /// bottom, nearest the orb).
    pub(crate) down: bool,
    /// The rows' greatest height; they scroll past it.
    pub(crate) max_h: f32,
    /// How a row comes and goes.
    pub(crate) appear: rune::Transition,
    /// Hold the minis' "Live" pulse still (Reduce motion, or a render).
    pub(crate) still: bool,
    /// The active project's name: a row in another project wears a tag.
    pub(crate) active_project: Option<String>,
}

/// The rows (and the "more" line) for `jobs`, as laid out: one column that
/// takes the pointer, so the wheel and the gaps between rows answer, and
/// scrolls should it outgrow its room. `team` is every helper on the board
/// (any phase), for its lead's row. `desk` (the person's desk at the orb,
/// [`super::minis`]) follows the rows in the same column, so everything
/// with live pictures scrolls together inside the panel.
#[allow(clippy::too_many_arguments)]
pub(crate) fn view<S: 'static>(
    p: Paint,
    jobs: &[&Task],
    team: &[&Task],
    layout: &Layout,
    room: Room,
    desk: Option<El>,
    actions: stack::Actions<S>,
    line_actions: LineActions<S>,
) -> Option<El> {
    if layout.rows.is_empty() && layout.more == 0 && desk.is_none() {
        return None;
    }
    let shown: Vec<&Task> = layout
        .rows
        .iter()
        .filter_map(|id| jobs.iter().find(|task| &task.id == id).copied())
        .collect();
    // Jobs of more than one project: a group per project, under its
    // header; one group looks as it always did.
    let groups = groups(&shown);
    let grouped = groups.len() > 1;
    let mut units: Vec<(Option<El>, Vec<El>)> = groups
        .into_iter()
        .map(|(name, tasks)| {
            let header = grouped.then(|| {
                let count = jobs
                    .iter()
                    .filter(|task| task.project.as_deref() == name)
                    .count();
                super::project::header(p, name, count).transition(room.appear)
            });
            let cards = tasks
                .into_iter()
                .map(|task| {
                    // Alone, a job in another project than the active one
                    // wears its tag; grouped, the header names it.
                    let tag = task
                        .project
                        .as_deref()
                        .filter(|name| !grouped && Some(*name) != room.active_project.as_deref());
                    card(
                        p,
                        task,
                        &app_live::previews(&task.id),
                        &live::previews(&task.id),
                        &helpers_of(team, &task.id),
                        actions,
                        line_actions.wall,
                        room.still,
                        tag,
                    )
                    .transition(room.appear)
                })
                .collect();
            (header, cards)
        })
        .collect();
    let mut rows: Vec<El> = Vec::new();
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
    rows.extend(desk.map(|desk| desk.transition(room.appear)));
    // Top to bottom: downward, the groups then the rest; upward (the newest
    // nearest the orb at the bottom), all reversed, each header still over
    // its group.
    if !room.down {
        rows.reverse();
        units.reverse();
        for (_, cards) in &mut units {
            cards.reverse();
        }
    }
    let groups: Vec<El> = units
        .into_iter()
        .flat_map(|(header, cards)| header.into_iter().chain(cards))
        .collect();
    let rows: Vec<El> = if room.down {
        groups.into_iter().chain(rows).collect()
    } else {
        rows.into_iter().chain(groups).collect()
    };
    Some(
        col()
            .key("jobs")
            .w(CARD_W + 2.0 * SHADOW_ROOM)
            .px(SHADOW_ROOM)
            .ml(-SHADOW_ROOM)
            .mr(-SHADOW_ROOM)
            .pb(SHADOW_ROOM)
            .mb(-SHADOW_ROOM)
            .max_h(room.max_h + SHADOW_ROOM)
            .min_h(0.0)
            .shrink()
            .gap(5.0)
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

/// `jobs` by project, in the order each project first shows.
pub(crate) fn groups<'a>(jobs: &[&'a Task]) -> Vec<(Option<&'a str>, Vec<&'a Task>)> {
    let mut groups: Vec<(Option<&str>, Vec<&Task>)> = Vec::new();
    for task in jobs {
        let name = task.project.as_deref();
        match groups.iter_mut().find(|(group, _)| *group == name) {
            Some((_, tasks)) => tasks.push(task),
            None => groups.push((name, vec![task])),
        }
    }
    groups
}

/// One job's row in the rail, in the card material: its dot (with its
/// plan's ring), title, "All N live", and how long it runs; one line of
/// where it works and what it does now ("Safari · in background ·
/// choosing a time"); its browsers and windows, live; its helpers; and a
/// thin plan bar. A press opens the task list with its plan, Take over,
/// and Stop.
#[allow(clippy::too_many_arguments)]
fn card<S: 'static>(
    p: Paint,
    task: &Task,
    apps: &[std::sync::Arc<AppPreview>],
    previews: &[std::sync::Arc<Preview>],
    helpers: &[&Task],
    actions: stack::Actions<S>,
    open_wall: fn(&mut S, &str),
    still: bool,
    project: Option<&str>,
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
            .map(|helper| super::pictures::count(&helper.id))
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
                .child(text(label).font(look::meta(p)).color(p.text())),
        )
    });
    let when = if task.phase == Phase::Queued {
        None
    } else {
        minutes(task.since, Instant::now()).map(|minutes| {
            text(running_for(minutes))
                .key("job-time")
                .font(look::meta(p))
                .color(p.faint())
        })
    };
    let head = row()
        .key("job-head")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(orb::marker(p, task, colour, 9.0))
        .child(
            text(task.title.clone())
                .key("job-title")
                .truncate()
                .font(look::label(p))
                .color(p.text())
                .min_w(0.0)
                .grow(1.0),
        )
        .children(project.map(|name| super::project::tag(p, name)))
        .children(
            // Its project's dot, where no tag says it.
            task.project
                .as_deref()
                .filter(|_| project.is_none())
                .map(|name| {
                    super::project::mark(p, Some(name), 6.0)
                        .key("job-project-dot")
                        .a11y_hidden()
                }),
        )
        .children(wall)
        .children(when);
    let step = doing(task).map(|words| {
        text(words)
            .key(if task.phase == Phase::Queued {
                "job-queued"
            } else {
                "job-step"
            })
            .truncate()
            .font(look::meta(p))
            .color(p.muted())
            .min_w(0.0)
            .pl(STEP_INSET)
    });
    let browsers = (apps_shown + shown > 0).then(|| {
        let mut cells: Vec<El> = apps[..apps_shown]
            .iter()
            .map(|preview| app_thumb(p, task, preview, still, actions))
            .chain(
                previews[..shown]
                    .iter()
                    .map(|preview| thumb(p, task, preview, still, actions)),
            )
            .collect();
        // The rest as "+N" on the last picture; a press shows them all.
        if more > 0
            && let Some(last) = cells.pop()
        {
            let id = task.id.clone();
            let badge = p.k.interactive(
                el().key("job-more")
                    .absolute(THUMB_W - 36.0, THUMB_H - 24.0)
                    .h(20.0)
                    .min_w(32.0)
                    .px(6.0)
                    .radius(10.0)
                    .fill(c.bg_base.alpha(0.92))
                    .border(p.hair, c.border_base)
                    .center()
                    .role("button")
                    .label(tr_fill("All {} live", &all.to_string()))
                    .hit()
                    .on_press(move |s: &mut S| open_wall(s, &id))
                    .child(
                        text(format!("+{more}"))
                            .font(look::label(p))
                            .color(p.text()),
                    ),
            );
            cells.push(el().key("job-last").child(last).child(badge));
        }
        row()
            .key("job-browsers")
            .gap(THUMB_GAP)
            .pt(2.0)
            .pl(STEP_INSET)
            .role("list")
            .label(tr("Live windows"))
            .children(cells)
    });
    let team = (!helpers.is_empty()).then(|| team_row(p, helpers).pl(STEP_INSET));
    let plan = task.progress().map(|(done, all)| {
        el().pl(STEP_INSET).child(look::progress(
            p,
            colour,
            done as f32 / all.max(1) as f32,
            CARD_W - 2.0 * CARD_PAD - STEP_INSET,
        ))
    });
    let card = look::material(
        p,
        col()
            .key(format!("job-{}", task.id))
            .w(CARD_W)
            .padding(CARD_PAD)
            .gap(4.0),
    )
    .role("button")
    .label(card_label(task))
    .hit()
    .on_press(move |s: &mut S| toggle(s))
    .child(head)
    .children(step)
    .children(browsers)
    .children(team)
    .children(plan);
    p.k.interactive(card)
}

/// Room around the rows inside their scrolling column, so the cards'
/// shadows show instead of ending at its edge.
const SHADOW_ROOM: f32 = 10.0;
/// Room inside a job's row.
const CARD_PAD: f32 = 11.0;
/// Between a row's pictures.
const THUMB_GAP: f32 = 6.0;
/// How far a row's step, pictures, and helpers are set in: past its dot.
const STEP_INSET: f32 = 17.0;

/// The app a job works in (its first window's), else the site its browser
/// is on, else "Browser" for a job on the web.
pub(crate) fn app_of(task: &Task) -> Option<String> {
    task.windows
        .first()
        .map(|window| window.app.clone())
        .or_else(|| {
            live::previews(&task.id)
                .first()
                .and_then(|preview| live::host_of(&preview.page().url))
        })
        .or_else(|| (task.place() == super::board::Place::Web).then(|| tr("Browser").to_owned()))
}

/// Where a job works and how, as its row says it: the app (or the site)
/// and whether it's in the background, queued, or yours ("Safari · in
/// background"); `None` when there's nothing to say. A wait for a usage
/// limit has its own words (its standing bubble, the list's line).
pub(crate) fn whereabouts(task: &Task) -> Option<String> {
    let app = app_of(task);
    let how = match task.phase {
        Phase::Queued => Some(tr("queued").to_owned()),
        Phase::Paused => Some(tr("you have it").to_owned()),
        Phase::Running => app.is_some().then(|| tr("in background").to_owned()),
        _ => None,
    };
    // The state alone starts the line: in capitals ("Queued"); an app or
    // a site keeps its own spelling.
    match app {
        Some(app) => Some(how.map_or(app.clone(), |how| format!("{app} · {how}"))),
        None => how.map(|how| capitalised(&how)),
    }
}

/// What a job does now, in its plan's words (or its latest step, or
/// Thinking…): the step its row's line ends with.
pub(crate) fn now_step(task: &Task) -> Option<String> {
    if let Some(at) = task.current_step()
        && let Some(step) = task.plan.get(at)
    {
        return Some(step.title.trim().to_owned());
    }
    task.step
        .clone()
        .or_else(|| (task.phase == Phase::Running).then(|| tr("Thinking…").to_owned()))
}

/// A row's one line: [`whereabouts`] and [`now_step`], "Safari · in
/// background · choosing a time".
pub(crate) fn doing(task: &Task) -> Option<String> {
    let parts: Vec<String> = whereabouts(task)
        .into_iter()
        .chain(now_step(task).filter(|_| task.phase == Phase::Running))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `words` with its first letter in capitals ("queued" alone reads
/// "Queued").
fn capitalised(words: &str) -> String {
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
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

/// A mini: its picture (or, without one, `look::no_picture`) with its
/// liveness badge in the corner ("● Live" pulsing while frames come, "8 s
/// ago" once they stall), in a frame `key`ed for the ring around it.
pub(crate) fn mini(
    p: Paint,
    key: String,
    pixels: &rune::Pixels,
    liveness: look::Liveness,
    fallback: (&'static rune::Icon, &str),
    still: bool,
) -> El {
    let c = &p.k.c;
    let picture = if pixels.size() != (0, 0) {
        el().key(key)
            .size(THUMB_W, THUMB_H)
            .radius(8.0)
            .fill(c.layer_02)
            .border(p.hair, p.k.alpha(c.text_base, 0.14))
            .pixels_fit(pixels, rune::SurfaceFit::Fill)
    } else {
        el().key(key).child(look::no_picture(
            p,
            fallback.0,
            fallback.1,
            liveness,
            (THUMB_W, THUMB_H),
        ))
    };
    el().size(THUMB_W, THUMB_H)
        .child(picture)
        .children(look::live_badge(p, liveness, still).map(|badge| badge.absolute(6.0, 6.0)))
}

/// One browser on a card, live from its thumbnail stream; a press opens it
/// large in the overlay.
fn thumb<S: 'static>(
    p: Paint,
    task: &Task,
    preview: &Preview,
    still: bool,
    actions: stack::Actions<S>,
) -> El {
    let page = preview.page();
    let liveness = look::liveness(
        preview.clock.age(preview.has_frame()),
        page.failed.is_some(),
    );
    // Its site under it, so a row says where it works.
    let name = live::host_of(&page.url).unwrap_or_else(|| {
        if preview.name.is_empty() {
            tr("Main browser").to_owned()
        } else {
            preview.name.clone()
        }
    });
    let title = if page.title.is_empty() {
        name.clone()
    } else {
        page.title.clone()
    };
    let ring = super::pictures::Ring::of(task, Instant::now());
    let picture = super::pictures::ring(
        p,
        ring,
        palette::task(task.colour, p.k.dark),
        mini(
            p,
            format!("job-thumb-{}", preview.session),
            &preview.pixels,
            liveness,
            (&GLOBE, &title),
            still,
        ),
        (THUMB_W, THUMB_H),
    );
    let open = actions.open_preview;
    let spoken = super::pictures::spoken(&name, &task.title, ring);
    let (task, session) = (task.id.clone(), preview.session.clone());
    let captioned = col()
        .key(format!("job-thumb-cell-{}", preview.session))
        .w(THUMB_W)
        .gap(5.0)
        .child(picture)
        .child(
            text(super::pictures::caption(&name, ring))
                .key("job-thumb-name")
                .truncate()
                .font(look::meta(p))
                .color(p.muted())
                .min_w(0.0),
        );
    p.k.interactive(
        captioned
            .role("button")
            .label(format!("{}: {spoken}", tr_fill("Open {} live", &name)))
            .hit()
            .on_press(move |s: &mut S| open(s, &task, &session)),
    )
}

/// One app window a job works in, live from its window (even parked off
/// the screen), with the app's name; a press opens it large in the overlay.
fn app_thumb<S: 'static>(
    p: Paint,
    task: &Task,
    preview: &AppPreview,
    still: bool,
    actions: stack::Actions<S>,
) -> El {
    let liveness = look::liveness(preview.clock.age(preview.has_frame()), preview.failed());
    let name = preview.window.app.clone();
    let ring = super::pictures::Ring::of(task, Instant::now());
    let picture = super::pictures::ring(
        p,
        ring,
        palette::task(task.colour, p.k.dark),
        mini(
            p,
            format!("job-app-{}", preview.number),
            &preview.pixels,
            liveness,
            (&PANEL, &name),
            still,
        ),
        (THUMB_W, THUMB_H),
    );
    let open = actions.open_preview;
    let spoken = super::pictures::spoken(&name, &task.title, ring);
    let (task, session) = (task.id.clone(), preview.session());
    let captioned = col()
        .key(format!("job-app-cell-{}", preview.number))
        .w(THUMB_W)
        .gap(5.0)
        .child(picture)
        .child(
            text(super::pictures::caption(&name, ring))
                .key("job-app-name")
                .truncate()
                .font(look::meta(p))
                .color(p.muted())
                .min_w(0.0),
        );
    p.k.interactive(
        captioned
            .role("button")
            .label(format!("{}: {spoken}", tr_fill("Open {} live", &name)))
            .hit()
            .on_press(move |s: &mut S| open(s, &task, &session)),
    )
}

/// Whether the rail shows live pictures now (the list folded, a job row
/// with a browser or window): it then looks again each `look::LIVE_TICK`,
/// so a stalled stream says so.
pub(crate) fn watching(board: &super::board::Board, open: bool) -> bool {
    orb::carded(board, open)
        .iter()
        .any(|id| !live::previews(id).is_empty() || !app_live::previews(id).is_empty())
}
/// The jobs past the rows: one "N more" line with each one's dot; a press
/// (or its list button) opens the task list in place, where every job is.
fn line<S: 'static>(p: Paint, rest: &[&Task], count: usize, actions: stack::Actions<S>) -> El {
    let toggle = actions.toggle;
    let shown_dots = 6;
    let more_dots = rest.len().saturating_sub(shown_dots);
    let label = tr_fill("{} more", &count.to_string());
    look::material(
        p,
        row()
            .key("jobs-line")
            .w(CARD_W)
            .h(LINE_H)
            .pl(14.0)
            .pr(4.0)
            .gap(6.0)
            .center_y(),
    )
    .radius(LINE_H / 2.0)
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
            .font(look::meta(p))
            .color(p.faint())
    }))
    .child(
        text(label)
            .key("jobs-line-text")
            .font(look::label(p))
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
    fn a_row_says_where_a_job_works_and_what_it_does_now() {
        use crate::assistant::TaskWindow;
        let board = board_with(1, false);
        let mut task = board.tasks[0].clone();
        task.phase = Phase::Running;
        task.step = Some("Choosing a time".into());
        assert_eq!(doing(&task).as_deref(), Some("Choosing a time"));
        task.windows = vec![TaskWindow {
            app: "Safari".into(),
            window_id: None,
        }];
        assert_eq!(
            doing(&task).as_deref(),
            Some("Safari · in background · Choosing a time")
        );
        task.phase = Phase::Paused;
        assert_eq!(whereabouts(&task).as_deref(), Some("Safari · you have it"));
        assert_eq!(doing(&task).as_deref(), Some("Safari · you have it"));
        task.phase = Phase::Queued;
        assert_eq!(doing(&task).as_deref(), Some("Safari · queued"));
        task.windows.clear();
        assert_eq!(
            doing(&task).as_deref(),
            Some("Queued"),
            "alone, capitalised"
        );
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
    fn a_card_shows_two_large_pictures_apps_first_and_counts_the_rest() {
        // Browsers alone.
        assert_eq!(strip(0, 0), (0, 0, 0));
        assert_eq!(strip(0, 2), (0, 2, 0));
        assert_eq!(strip(0, 12), (0, 2, 10));
        // App windows first, browsers in the room left, one strip of two.
        assert_eq!(strip(1, 2), (1, 1, 1));
        assert_eq!(strip(2, 5), (2, 0, 5));
        assert_eq!(strip(6, 1), (2, 0, 5));
        // Two pictures fill a row's strip, set in past its dot, each large
        // enough for its motion to read.
        const { assert!(THUMB_W >= 140.0 && THUMB_H >= 85.0) };
        let strip_w = MAX_THUMBS as f32 * THUMB_W + (MAX_THUMBS - 1) as f32 * THUMB_GAP;
        assert!(strip_w <= CARD_W - 2.0 * CARD_PAD - STEP_INSET, "{strip_w}");
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
