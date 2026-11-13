//! The orb: Background tasks' quiet presence. While any runs, a small
//! glowing orb floats beside the HUD (or wherever the person dragged it on
//! that display), breathing, with one tiny dot per task in its colour: full
//! while it runs, a ring while the person has it, faint while it waits.
//!
//! Beside it, one calm column, nearest the orb first, so nothing overlaps
//! or jumps:
//!
//! - The question to answer now (the inbox's active one,
//!   `Event::QuestionsChanged`), in its task's colour: who asks, the
//!   question, and how to answer, by voice ("Hold ⌘ to answer") or typed
//!   right there ([`super::ask`]); Later puts it behind the others. The orb
//!   wears a small count of every open question.
//! - A task the person took over keeps "You have it" with Hand back; one
//!   waiting out a usage limit keeps "Waiting for limit · resumes 14:05".
//! - The running jobs as a few compact rows with their step and live
//!   pictures, the rest summed up in one line ([`super::jobs`]).
//! - A finished job's result, which fades, with what it names to open and
//!   Try again on a failure ([`super::results`]).
//!
//! A task with a plan wears a small progress ring around its dot. A Here
//! question that turns into a long job (`Event::Escalated`) flies into the
//! cluster as a new dot. After the last job ends the orb stays, quieter,
//! while it can recall a result.
//!
//! The full list ([`super::stack`]) unfolds from the orb on a press, a
//! double-tap of the Action shortcut, or a press on a job's row, and folds
//! back on ×, Escape, a click outside, or another press. Everything grows
//! from the orb's side of the panel with springs, and nothing but the orb
//! and its bubbles takes clicks.

use rune::{Cx, El, LineCap, Shape, Transition, Vec2, canvas, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};
use rune_motion::{LoopSpec, Spring};

use super::super::controls::Paint;
use super::ask::{self, Reply};
use super::board::{Board, Phase, Task};
use super::pops::MAX_POPS;
use super::results::{self, Outcome};
use super::{frame, jobs, palette, stack, when};
use crate::desktop::i18n::{tr, tr_fill};

/// The panel: room for the list unfolded, or a few bubbles.
pub(crate) const SIZE: (f32, f32) = (400.0, 600.0);
/// Room around everything inside the panel.
pub(crate) const INSET: f32 = 10.0;
/// The orb's height (it's as wide as its dots need).
pub(crate) const ORB_H: f32 = 30.0;
/// The glowing core's size.
const CORE: f32 = 14.0;
/// A task's dot.
const DOT: f32 = 7.0;
/// The most dots the orb wears; more live tasks show as "+N".
pub(crate) const MAX_DOTS: usize = 10;
const GAP: f32 = 8.0;
/// A bubble's widest.
const BUBBLE_MAX_W: f32 = 320.0;
const BUBBLE_H: f32 = 30.0;
/// A job row's height without pictures: its head and step.
const ROW_MIN: f32 = 54.0;
/// A progress ring around a dot: its size and stroke.
const RING: f32 = 13.0;
const RING_W: f32 = 1.75;

/// Which way things unfold from the orb, from where it sits on its display.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Grow {
    /// Downward (the orb is in the display's top half), else upward.
    pub(crate) down: bool,
    /// Rightward (the orb is left of the display's right quarter), else
    /// leftward.
    pub(crate) right: bool,
}

impl Default for Grow {
    fn default() -> Self {
        Self {
            down: true,
            right: true,
        }
    }
}

/// What a press or gesture asks of the host.
pub(crate) struct Actions<S> {
    pub(crate) list: stack::Actions<S>,
    /// The pointer left the orb and what unfolds from it (`false`), or came
    /// onto it.
    pub(crate) hover: fn(&mut S, bool),
    /// The orb is dragged somewhere else.
    pub(crate) drag: fn(&mut S, &rune::Drag),
    /// A job's row: its live wall.
    pub(crate) jobs: jobs::LineActions<S>,
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

impl<S: 'static> Actions<S> {
    /// Presses that do nothing, for drawings of the orb (the playground's
    /// scenes and the render tests); a scene that types an answer brings
    /// its own `list.reply`.
    pub(crate) fn inert() -> Self {
        Self {
            list: stack::Actions {
                stop: |_, _| {},
                take_over: |_, _| {},
                hand_back: |_, _| {},
                stop_all: |_| {},
                toggle: |_| {},
                dismiss: |_, _| {},
                hover: |_, _, _| {},
                open_preview: |_, _, _| {},
                defer: |_, _| {},
                reply: ask::ReplyActions {
                    start: |_, _| {},
                    cancel: |_| {},
                    send: |_| {},
                    edit: |_| unreachable!("a drawing without an answer field"),
                },
                results: results::Actions {
                    open: |_, _| {},
                    retry: |_, _| {},
                    dismiss: |_, _| {},
                    expand: |_, _| {},
                    recall: |_, _| {},
                },
            },
            hover: |_, _| {},
            drag: |_, _| {},
            jobs: jobs::LineActions { wall: |_, _| {} },
        }
    }
}

/// The orb panel's state.
pub(crate) struct Orb {
    /// Every task (the outlines read it too), the bubbles, and the results.
    pub(crate) board: Board,
    /// Whether the panel shows.
    pub(crate) shown: bool,
    /// The departure (a finished task, a bubble, a countdown's minute) the
    /// panel is woken for.
    pub(crate) armed: Option<std::time::Instant>,
    /// The list is unfolded (a press, or `toggle_tasks`).
    pub(crate) expanded: bool,
    /// The task whose row the pointer rests on: its browsers stream live.
    pub(crate) hovered: Option<String>,
    /// A typed answer to the question, while its field shows.
    pub(crate) reply: Option<Reply>,
    /// The result bubble unfolded to its whole answer (its key).
    pub(crate) open_result: Option<u64>,
    pub(crate) grow: Grow,
    /// The Action shortcut's keys, for "Hold ⌘ to answer".
    pub(crate) answer_key: String,
    /// How loud an answer read aloud sounds now (0 to 1), when it can be
    /// measured: the core pulses with it.
    pub(crate) voice: Option<f32>,
    pub(crate) actions: Actions<Orb>,
}

impl Orb {
    /// A hidden, folded orb over `board`.
    pub(crate) fn new(board: Board, answer_key: String, actions: Actions<Orb>) -> Self {
        Self {
            board,
            shown: false,
            armed: None,
            expanded: false,
            hovered: None,
            reply: None,
            open_result: None,
            grow: Grow::default(),
            answer_key,
            voice: None,
            actions,
        }
    }
}

/// What the view needs besides the board.
#[derive(Clone, Debug, Default)]
pub(crate) struct Look {
    pub(crate) open: bool,
    pub(crate) grow: Grow,
    /// Hold the breathing still (Reduce motion, or a render).
    pub(crate) still: bool,
    /// The Action shortcut's keys, for "Hold ⌘ to answer".
    pub(crate) answer_key: String,
    /// The voice reading an answer aloud, when it can be measured.
    pub(crate) voice: Option<f32>,
    /// The task whose row the pointer rests on.
    pub(crate) hovered: Option<String>,
    /// A typed answer under way.
    pub(crate) reply: Option<Reply>,
    /// The result bubble unfolded (its key).
    pub(crate) open_result: Option<u64>,
}

pub(crate) fn view(orb: &Orb, cx: &mut Cx<Orb>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let look = Look {
        open: orb.expanded,
        grow: orb.grow,
        still: cx.system_reduced_motion(),
        answer_key: orb.answer_key.clone(),
        voice: orb.voice,
        hovered: orb.hovered.clone(),
        reply: orb.reply.clone(),
        open_result: orb.open_result,
    };
    el().key("orb-root")
        .w_full()
        .h_full()
        .padding(INSET)
        .hover_motion_within(rune::Anim::INSTANT, rune::Anim::INSTANT)
        .child(content(p, &orb.board, &look, orb.actions))
}

/// The jobs whose rows show at the orb with the list folded (`open`
/// false): their browsers and windows stream.
pub(crate) fn carded(board: &Board, open: bool) -> Vec<String> {
    if open {
        return Vec::new();
    }
    let running = jobs::running(&board.tasks);
    jobs::layout(&running, board.active_question().is_some()).rows
}

/// Whether the list has anything to show: a job, or a result.
pub(crate) fn listable(board: &Board) -> bool {
    !board.stack().is_empty() || !board.recent.is_empty()
}

/// The result bubbles up now, newest first: the unfolded one alone while
/// one is, else the newest [`MAX_POPS`], each with its result.
pub(crate) fn result_bubbles(board: &Board, open: Option<u64>) -> Vec<(u64, Outcome)> {
    let mut items: Vec<(u64, Outcome)> = board
        .pops
        .items
        .iter()
        .rev()
        .take(MAX_POPS)
        .map(|pop| {
            let outcome = board
                .outcome(&pop.task)
                .cloned()
                .unwrap_or_else(|| Outcome {
                    id: pop.task.clone(),
                    title: board
                        .task(&pop.task)
                        .map(|task| task.title.clone())
                        .unwrap_or_default(),
                    colour: pop.colour,
                    failed: pop.kind == super::pops::PopKind::Failed,
                    text: pop.text.clone(),
                    at: std::time::Instant::now(),
                });
            (pop.key, outcome)
        })
        .collect();
    if let Some(open) = open
        && items.iter().any(|(key, _)| *key == open)
    {
        items.retain(|(key, _)| *key == open);
    }
    items
}

/// The orb and what unfolds from it, filling its parent from the orb's
/// corner.
pub(crate) fn content<S: 'static>(p: Paint, board: &Board, look: &Look, actions: Actions<S>) -> El {
    let tasks = board.stack();
    let done = board.results();
    let grow = look.grow;
    let mut parts: Vec<El> = Vec::new();
    let desk = super::minis::any();
    if !tasks.is_empty() || !board.pops.items.is_empty() || !done.is_empty() || desk {
        parts.push(orb(
            p,
            &tasks,
            done.len(),
            board.open_questions(),
            look,
            actions,
        ));
    }
    if look.open && (!tasks.is_empty() || !done.is_empty()) {
        let max_h = SIZE.1 - 2.0 * INSET - ORB_H - GAP;
        parts.push(
            stack::sheet(
                p,
                &tasks,
                &done,
                grow.down,
                max_h,
                look.hovered.as_deref(),
                actions.list,
            )
            .transition(unfold(grow)),
        );
    } else {
        let asking = board.active_question();
        let replying = asking.as_ref().is_some_and(|asking| {
            look.reply
                .as_ref()
                .is_some_and(|reply| reply.id == asking.id)
        });
        let mut bubbles: Vec<El> = asking
            .as_ref()
            .map(|asking| {
                ask::question(
                    p,
                    asking,
                    &look.answer_key,
                    look.reply.as_ref(),
                    actions.list,
                )
                .transition(pop_in(grow))
            })
            .into_iter()
            .collect();
        // Each task the person holds keeps its bubble (with Hand back);
        // several waiting out a usage limit share one, so a limit that
        // stops every job at once doesn't run past the panel.
        let waiting: Vec<&Task> = tasks
            .iter()
            .filter(|task| matches!(task.phase, Phase::Waiting { .. }))
            .collect();
        let mut standing: Vec<El> = Vec::new();
        if waiting.len() > 1 {
            standing.extend(
                tasks
                    .iter()
                    .filter(|task| !matches!(task.phase, Phase::Waiting { .. }))
                    .filter_map(|task| standing_bubble(p, task, grow, actions.list)),
            );
            standing.push(all_waiting(p, &waiting).transition(pop_in(grow)));
        } else {
            standing.extend(
                tasks
                    .iter()
                    .filter_map(|task| standing_bubble(p, task, grow, actions.list)),
            );
        }
        let running = jobs::running(&board.tasks);
        let layout = jobs::layout(&running, asking.is_some());
        let mut results = result_bubbles(board, look.open_result);
        // What else takes room in the panel, so the job rows take the rest
        // and scroll past it instead of pushing the results off the panel.
        let fixed = ORB_H
            + GAP
            + if asking.is_some() {
                ask::ROOM + GAP + if replying { ask::FIELD_ROOM } else { 0.0 }
            } else {
                0.0
            }
            + standing.len() as f32 * (BUBBLE_H + GAP);
        let results_room = |results: &[(u64, Outcome)]| {
            results
                .iter()
                .map(|(key, _)| {
                    GAP + if look.open_result == Some(*key) {
                        results::OPEN_ROOM
                    } else {
                        results::ROOM
                    }
                })
                .sum::<f32>()
        };
        // The rows keep room for two of them (and the "more" line): an
        // older result gives way first (the list still recalls it).
        let rows_need = layout.rows.len().min(2) as f32 * (ROW_MIN + GAP)
            + if layout.more > 0 {
                jobs::LINE_H + GAP
            } else {
                0.0
            };
        while results.len() > 1 && SIZE.1 - 2.0 * INSET - fixed - results_room(&results) < rows_need
        {
            // The oldest done one; a failure is the last to go.
            let at = results
                .iter()
                .rposition(|(_, outcome)| !outcome.failed)
                .unwrap_or(results.len() - 1);
            results.remove(at);
        }
        let taken = fixed + results_room(&results);
        bubbles.extend(standing);
        let room = jobs::Room {
            down: grow.down,
            max_h: (SIZE.1 - 2.0 * INSET - taken).max(jobs::LINE_H + 2.0 * GAP),
            appear: pop_in(grow),
        };
        bubbles.extend(
            jobs::view(
                p,
                &running,
                &board
                    .tasks
                    .iter()
                    .filter(|task| task.lead.is_some())
                    .collect::<Vec<_>>(),
                &layout,
                room,
                actions.list,
                actions.jobs,
            )
            .map(|rows| rows.transition(pop_in(grow))),
        );
        // The person's desk: what quick turns opened, live, until let go.
        bubbles.extend(
            super::minis::view(p, actions.list).map(|minis| minis.transition(pop_in(grow))),
        );
        bubbles.extend(results.iter().map(|(key, outcome)| {
            results::bubble(
                p,
                *key,
                outcome,
                look.open_result == Some(*key),
                actions.list.results,
            )
            .transition(pop_in(grow))
        }));
        parts.extend(bubbles);
    }
    if !grow.down {
        parts.reverse();
    }
    let hover = actions.hover;
    let cluster = col()
        .key("cluster")
        .gap(GAP)
        .hover_delays(
            std::time::Duration::from_millis(220),
            std::time::Duration::from_millis(450),
        )
        .on_hover_change(move |s: &mut S, inside, _| hover(s, inside))
        .animate_layout()
        .children(parts);
    let cluster = if grow.right {
        cluster.items_start()
    } else {
        cluster.items_end()
    };
    let root = col().key("orb-content").w_full().h_full().child(cluster);
    let root = if grow.down {
        root.justify_start()
    } else {
        root.justify_end()
    };
    if grow.right {
        root.items_start()
    } else {
        root.items_end()
    }
}

/// The orb: its glowing core, a dot per live task, how many questions wait
/// on the person, and, with nothing live, how many results it recalls.
fn orb<S: 'static>(
    p: Paint,
    tasks: &[Task],
    done: usize,
    questions: usize,
    look: &Look,
    actions: Actions<S>,
) -> El {
    let c = &p.k.c;
    let (lead, trail) = palette::frame(p.k.dark);
    let core = el()
        .key("core")
        .size(CORE, CORE)
        .radius(CORE / 2.0)
        .gradient(lead, trail)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 10.0,
            spread: 1.0,
            color: lead.alpha(0.6),
            inset: false,
        })
        .a11y_hidden();
    let live_tasks: Vec<&Task> = tasks.iter().filter(|task| task.phase.live()).collect();
    let core = match look.voice {
        // With the voice reading an answer aloud.
        Some(level) => core.opacity(0.5 + 0.5 * super::bubble::pulse(level)),
        // Nothing at work: still, and quieter.
        None if look.still || live_tasks.is_empty() => core,
        None => core.looping(LoopSpec::opacity(frame::BREATH, 0.0, 0.62, 1.0)),
    };
    // The newest few as dots; the rest as a count, so fifty jobs keep the
    // orb small.
    let hidden = live_tasks.len().saturating_sub(MAX_DOTS);
    let dots = live_tasks[hidden..].iter().map(|task| {
        let colour = palette::task(task.colour, p.k.dark);
        marker(p, task, colour, DOT)
            .key(format!("orb-dot-{}", task.id))
            .transition(fly_in(look.grow))
    });
    let mut items: Vec<El> = vec![core];
    items.extend((hidden > 0).then(|| {
        text(format!("+{hidden}"))
            .key("orb-more")
            .font(p.k.ty.small)
            .color(p.faint())
    }));
    items.extend(dots);
    // Nothing live: what it can recall, a press away.
    let recalling = live_tasks.is_empty() && done > 0;
    items.extend(recalling.then(|| {
        row()
            .key("orb-recent")
            .gap(3.0)
            .center_y()
            .child(
                icon(p.k.glyph.check_small, 12.0)
                    .color(p.muted())
                    .a11y_hidden(),
            )
            .child(
                text(done.to_string())
                    .font(p.k.ty.small_medium)
                    .color(p.muted()),
            )
    }));
    items.extend((questions > 0).then(|| badge(p, questions, look.grow)));
    if !look.grow.right {
        items.reverse();
    }
    let toggle = actions.list.toggle;
    let drag = actions.drag;
    let live = live_tasks.len();
    let mut label = tr_fill("Assistant tasks: {}", &live.to_string());
    if questions > 0 {
        label = format!(
            "{label} · {}",
            tr_fill("Open questions: {}", &questions.to_string())
        );
    }
    if recalling {
        label = format!(
            "{label} · {}",
            tr_fill("Recent results: {}", &done.to_string())
        );
    }
    let pill = row()
        .key("orb")
        .h(ORB_H)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(ORB_H / 2.0)
        .fill(c.bg_base.alpha(0.94))
        .border(p.hair, c.border_base)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 4.0,
            blur: 16.0,
            spread: 0.0,
            color: lead.alpha(if p.k.dark { 0.35 } else { 0.22 }),
            inset: false,
        })
        .role("button")
        .label(label)
        .hit()
        .on_press(move |s: &mut S| toggle(s))
        .drag_threshold(4.0)
        .on_drag(move |s: &mut S, d: &mut rune::Drag, _| drag(s, d))
        .children(items);
    if live == 0 && questions == 0 {
        pill.opacity(0.72)
    } else {
        pill
    }
}

/// A task's dot in `colour`: full while it runs, a ring while the person
/// has it, faint while it waits or queues, soft once it's over.
pub(crate) fn dot(task: &Task, colour: rune::Color, size: f32) -> El {
    let base = el().size(size, size).radius(size / 2.0).a11y_hidden();
    match task.phase {
        Phase::Running => base.fill(colour).box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 6.0,
            spread: 0.0,
            color: colour.alpha(0.55),
            inset: false,
        }),
        Phase::Paused => base.border(1.5, colour),
        Phase::NeedsInput => base.fill(colour.alpha(0.3)).border(1.5, colour),
        Phase::Waiting { .. } => base.fill(colour.alpha(0.35)),
        Phase::Queued => base.fill(colour.alpha(0.45)),
        Phase::Done | Phase::Failed | Phase::Stopped => base.fill(colour.alpha(0.4)),
    }
}

/// A task's dot, inside a ring that fills as its plan's steps get done.
pub(crate) fn marker(p: Paint, task: &Task, colour: rune::Color, size: f32) -> El {
    let Some((done, total)) = task.progress() else {
        return el().center().child(dot(task, colour, size));
    };
    el().size(RING, RING)
        .center()
        .label(format!("{done}/{total}"))
        .child(
            canvas(ring(done as f32 / total as f32, colour, p.k.c.border_base))
                .key("ring")
                .absolute(0.0, 0.0)
                .size(RING, RING)
                .a11y_hidden(),
        )
        .child(dot(task, colour, size - 2.0))
}

/// A ring `RING` points across: a faint track, and `fraction` of it in
/// `colour` clockwise from the top.
fn ring(fraction: f32, colour: rune::Color, track: rune::Color) -> Shape {
    let r = (RING - RING_W) / 2.0;
    let c = RING / 2.0;
    let point = |t: f32| {
        let a = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        [c + r * a.cos(), c + r * a.sin()]
    };
    const SEGMENTS: usize = 48;
    let track_points: Vec<[f32; 2]> = (0..SEGMENTS)
        .map(|i| point(i as f32 / SEGMENTS as f32))
        .collect();
    let shape = Shape::new().stroke_with(track_points, true, RING_W, LineCap::Round, track);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return shape;
    }
    let steps = ((SEGMENTS as f32 * fraction).ceil() as usize).max(2);
    let arc: Vec<[f32; 2]> = (0..=steps)
        .map(|i| point(fraction * i as f32 / steps as f32))
        .collect();
    shape.stroke_with(arc, false, RING_W, LineCap::Round, colour)
}

/// The open questions' count on the orb, in the accent colour; it pops
/// again whenever the count changes (a new question arrived).
fn badge(p: Paint, count: usize, grow: Grow) -> El {
    let accent = p.accent();
    text(count.to_string())
        .key(format!("questions-{count}"))
        .font(p.k.ty.small_medium)
        .color(p.k.c.bg_base)
        .min_w(16.0)
        .h(16.0)
        .px(4.0)
        .radius(8.0)
        .fill(accent)
        .align(rune::Align::Center)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 8.0,
            spread: 0.0,
            color: accent.alpha(0.55),
            inset: false,
        })
        .a11y_hidden()
        .transition(
            Transition::scale_anchor(0.4, anchor(grow))
                .allow_small_scale()
                .combined(Transition::opacity())
                .springs(rune::SpringPair::new(
                    Spring::new(0.42, 0.32),
                    Spring::new(0.28, 0.0),
                )),
        )
}

/// The bubble a task keeps while the person has it, or while it waits out
/// a usage limit.
fn standing_bubble<S: 'static>(
    p: Paint,
    task: &Task,
    grow: Grow,
    actions: stack::Actions<S>,
) -> Option<El> {
    let colour = palette::task(task.colour, p.k.dark);
    let (words, button) = match task.phase {
        Phase::Paused => {
            let hand_back = actions.hand_back;
            let id = task.id.clone();
            (
                format!("{} · {}", tr("You have it"), task.title),
                Some(
                    Button::new(tr("Hand back"))
                        .variant(ButtonVariant::Submit)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut S| hand_back(s, &id))
                        .build(p.k)
                        .key("hand-back"),
                ),
            )
        }
        Phase::Waiting { until_ms } => (
            tr_fill("Waiting for limit · resumes {}", &when::clock(until_ms)),
            None,
        ),
        _ => return None,
    };
    let waiting = button.is_none();
    let bubble = shell(p, format!("standing-{}", task.id))
        .pr(if waiting { 12.0 } else { 3.0 })
        .child(dot(task, colour, 8.0))
        .child(
            text(words)
                .key("standing-text")
                .truncate()
                .font(p.k.ty.small_medium)
                .color(if waiting { p.muted() } else { p.text() })
                .min_w(0.0)
                .shrink(),
        )
        .children(button);
    Some(
        if waiting {
            bubble.opacity(0.75)
        } else {
            bubble
        }
        .transition(pop_in(grow)),
    )
}

/// Several tasks waiting out a usage limit, as one bubble: how many, and
/// when the first resumes.
fn all_waiting(p: Paint, waiting: &[&Task]) -> El {
    let first = waiting
        .iter()
        .filter_map(|task| match task.phase {
            Phase::Waiting { until_ms } => Some(until_ms),
            _ => None,
        })
        .min()
        .unwrap_or_default();
    let words = format!(
        "{} · {}",
        tr_fill("{} jobs wait for the limit", &waiting.len().to_string()),
        tr_fill("resumes {}", &when::clock(first))
    );
    shell(p, "standing-waiting".into())
        .pr(12.0)
        .children(
            waiting
                .iter()
                .take(6)
                .map(|task| dot(task, palette::task(task.colour, p.k.dark), 8.0)),
        )
        .child(
            text(words)
                .key("standing-text")
                .truncate()
                .font(p.k.ty.small_medium)
                .color(p.muted())
                .min_w(0.0)
                .shrink(),
        )
        .opacity(0.75)
}

/// A bubble's capsule.
fn shell(p: Paint, key: String) -> El {
    let c = &p.k.c;
    row()
        .key(key)
        .h(BUBBLE_H)
        .max_w(BUBBLE_MAX_W)
        .pl(10.0)
        .gap(7.0)
        .center_y()
        .radius(BUBBLE_H / 2.0)
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
        .hit()
}

/// The point of the orb a bubble grows from, in the bubble's own unit
/// coordinates.
fn anchor(grow: Grow) -> Vec2 {
    Vec2::new(
        if grow.right { 0.0 } else { 1.0 },
        if grow.down { 0.0 } else { 1.0 },
    )
}

/// A bubble pops out of the orb with a little overshoot, and dissolves.
fn pop_in(grow: Grow) -> Transition {
    let toward = if grow.down { -10.0 } else { 10.0 };
    Transition::asymmetric(
        Transition::scale_anchor(0.55, anchor(grow))
            .allow_small_scale()
            .combined(Transition::offset(0.0, toward)),
        Transition::scale_anchor(0.8, anchor(grow))
            .allow_small_scale()
            .combined(Transition::opacity()),
    )
    .springs(rune::SpringPair::new(
        Spring::new(0.42, 0.32),
        Spring::new(0.28, 0.0),
    ))
}

/// A new dot flies into the cluster from the HUD's side with a little
/// overshoot (a Here question escalated, or a task started).
fn fly_in(grow: Grow) -> Transition {
    Transition::asymmetric(
        Transition::scale(0.5)
            .allow_small_scale()
            .combined(Transition::offset(
                if grow.right { -70.0 } else { 70.0 },
                6.0,
            )),
        Transition::scale(0.5).allow_small_scale(),
    )
    .springs(rune::SpringPair::new(
        Spring::new(0.55, 0.35),
        Spring::new(0.25, 0.0),
    ))
}

/// The list unfolds from the orb and folds back into it.
fn unfold(grow: Grow) -> Transition {
    Transition::scale_anchor(0.86, anchor(grow))
        .allow_small_scale()
        .combined(Transition::offset(
            0.0,
            if grow.down { -12.0 } else { 12.0 },
        ))
        .springs(rune::SpringPair::new(
            Spring::new(0.38, 0.18),
            Spring::new(0.26, 0.0),
        ))
}
