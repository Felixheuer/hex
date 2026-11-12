//! The orb: Background tasks' quiet presence. While any runs, a small
//! glowing orb floats beside the HUD (or wherever the person dragged it on
//! that display), breathing, with one tiny dot per task in its colour: full
//! while it runs, a ring while the person has it, faint while it waits.
//!
//! Things pop out of it and settle back: a new task's title, its latest
//! step as a brief toast, ✓ or ✗ when it ends ([`super::pops`]). A task the
//! person took over keeps a small "You have it" bubble with Hand back; one
//! waiting out a usage limit keeps "Waiting for limit · resumes 14:05".
//! The question to answer now (the inbox's active one,
//! `Event::QuestionsChanged`) stays in a bubble in its task's colour,
//! nearest the orb: who asks ("Einstein research asks"), the question, how
//! to answer ("Hold ⌘ to answer"), and Later, which puts it behind the
//! others. The orb wears a small count of every open question. A task with a plan wears a
//! small progress ring around its dot. A Here question that turns into a
//! long job (`Event::Escalated`) flies into the cluster as a new dot.
//!
//! The full list ([`super::stack`]) unfolds from the orb on a press, a
//! double-tap of the Action shortcut, or while the pointer rests on it, and
//! folds back. Everything grows from the orb's side of the panel with
//! springs, and nothing but the orb and its bubbles takes clicks.

use rune::{Cx, El, LineCap, Shape, Transition, Vec2, canvas, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};
use rune_motion::{LoopSpec, Spring};

use super::super::controls::Paint;
use super::super::icons::SPARKLES;
use super::board::{Asking, Board, Phase, Task};
use super::pops::{Pop, PopKind};
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
const GAP: f32 = 8.0;
/// A bubble's widest.
const BUBBLE_MAX_W: f32 = 320.0;
const BUBBLE_H: f32 = 30.0;
/// A question's most lines.
const QUESTION_LINES: usize = 3;
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
    /// The pointer rests on the orb or its list (`true`), or left.
    pub(crate) hover: fn(&mut S, bool),
    /// The orb is dragged somewhere else.
    pub(crate) drag: fn(&mut S, &rune::Drag),
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

/// The orb panel's state.
pub(crate) struct Orb {
    /// Every task (the outlines read it too) and the bubbles.
    pub(crate) board: Board,
    /// Whether the panel shows.
    pub(crate) shown: bool,
    /// The departure (a finished task, a bubble, a countdown's minute) the
    /// panel is woken for.
    pub(crate) armed: Option<std::time::Instant>,
    /// The list is unfolded (a press, or `toggle_tasks`).
    pub(crate) expanded: bool,
    /// The pointer rests on the orb: the list peeks open.
    pub(crate) peek: bool,
    /// The task whose row the pointer rests on: its browsers stream live.
    pub(crate) hovered: Option<String>,
    pub(crate) grow: Grow,
    /// The Action shortcut's keys, for "Hold ⌘ to answer".
    pub(crate) answer_key: String,
    /// How loud an answer read aloud sounds now (0 to 1), when it can be
    /// measured: the core pulses with it.
    pub(crate) voice: Option<f32>,
    pub(crate) actions: Actions<Orb>,
}

/// What the view needs besides the board.
#[derive(Clone, Debug)]
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
}

pub(crate) fn view(orb: &Orb, cx: &mut Cx<Orb>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let look = Look {
        open: orb.expanded || orb.peek,
        grow: orb.grow,
        still: cx.system_reduced_motion(),
        answer_key: orb.answer_key.clone(),
        voice: orb.voice,
        hovered: orb.hovered.clone(),
    };
    el().key("orb-root")
        .w_full()
        .h_full()
        .padding(INSET)
        .hover_motion_within(rune::Anim::INSTANT, rune::Anim::INSTANT)
        .child(content(p, &orb.board, &look, orb.actions))
}

/// The orb and what unfolds from it, filling its parent from the orb's
/// corner.
pub(crate) fn content<S: 'static>(p: Paint, board: &Board, look: &Look, actions: Actions<S>) -> El {
    let tasks = board.stack();
    let grow = look.grow;
    let mut parts: Vec<El> = Vec::new();
    if !tasks.is_empty() || !board.pops.items.is_empty() {
        parts.push(orb(p, &tasks, board.open_questions(), look, actions));
    }
    if look.open && !tasks.is_empty() {
        let max_h = SIZE.1 - 2.0 * INSET - ORB_H - GAP;
        parts.push(
            stack::sheet(
                p,
                &tasks,
                grow.down,
                max_h,
                look.hovered.as_deref(),
                actions.list,
            )
            .transition(unfold(grow)),
        );
    } else {
        // One order, nearest the orb first, so nothing overlaps: the
        // question, a held or waiting task, the jobs, then the pops.
        let asking = board.active_question();
        let mut bubbles: Vec<El> = asking
            .as_ref()
            .map(|asking| {
                question(p, asking, &look.answer_key, actions.list).transition(pop_in(grow))
            })
            .into_iter()
            .collect();
        bubbles.extend(
            tasks
                .iter()
                .filter_map(|task| standing(p, task, grow, actions.list)),
        );
        let running = jobs::running(&board.tasks);
        let layout = jobs::layout(&running, asking.is_some());
        let carded: Vec<&str> = match &layout {
            jobs::Layout::Cards(ids) => ids.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        bubbles.extend(
            jobs::view(p, &running, &layout, actions.list)
                .into_iter()
                .map(|card| card.transition(pop_in(grow))),
        );
        bubbles.extend(
            board
                .pops
                .items
                .iter()
                .rev()
                // A job with a card shows its start and steps there.
                .filter(|item| {
                    !(matches!(item.kind, PopKind::New | PopKind::Step)
                        && carded.contains(&item.task.as_str()))
                })
                .map(|item| pop(p, item, actions.list).transition(pop_in(grow))),
        );
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

/// The orb: its glowing core, a dot per live task, and how many questions
/// wait on the person.
fn orb<S: 'static>(
    p: Paint,
    tasks: &[Task],
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
    let core = match look.voice {
        // With the voice reading an answer aloud.
        Some(level) => core.opacity(0.5 + 0.5 * super::bubble::pulse(level)),
        None if look.still => core,
        None => core.looping(LoopSpec::opacity(frame::BREATH, 0.0, 0.62, 1.0)),
    };
    let dots = tasks.iter().filter(|task| task.phase.live()).map(|task| {
        let colour = palette::task(task.colour, p.k.dark);
        marker(p, task, colour, DOT)
            .key(format!("orb-dot-{}", task.id))
            .transition(fly_in(look.grow))
    });
    let mut items: Vec<El> = vec![core];
    items.extend(dots);
    items.extend((questions > 0).then(|| badge(p, questions, look.grow)));
    if !look.grow.right {
        items.reverse();
    }
    let toggle = actions.list.toggle;
    let drag = actions.drag;
    let live = tasks.iter().filter(|task| task.phase.live()).count();
    let label = if questions > 0 {
        format!(
            "{} · {}",
            tr_fill("Assistant tasks: {}", &live.to_string()),
            tr_fill("Open questions: {}", &questions.to_string())
        )
    } else {
        tr_fill("Assistant tasks: {}", &live.to_string())
    };
    row()
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
        .children(items)
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

/// The question to answer now, in its task's colour: who asks, the
/// question, how to answer it, Later (behind the others), and × to put the
/// bubble away (it still waits for the answer). A press opens it in Chats.
fn question<S: 'static>(
    p: Paint,
    asking: &Asking,
    answer_key: &str,
    actions: stack::Actions<S>,
) -> El {
    let c = &p.k.c;
    let colour = palette::task(asking.colour, p.k.dark);
    let open = actions.open;
    let dismiss = actions.dismiss;
    let defer = actions.defer;
    let (id, put_away, later) = (asking.id.clone(), asking.id.clone(), asking.id.clone());
    let dot = el()
        .size(8.0, 8.0)
        .radius(4.0)
        .fill(colour.alpha(0.3))
        .border(1.5, colour)
        .a11y_hidden()
        .mt(5.0);
    row()
        .key(format!("question-{}", asking.id))
        .w(BUBBLE_MAX_W)
        .pl(12.0)
        .pr(4.0)
        .py(8.0)
        .gap(8.0)
        .items_start()
        .radius(16.0)
        .fill(c.bg_base)
        .border(1.0, colour.alpha(0.7))
        .outline(3.0, 0.0, colour.alpha(0.16))
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 3.0,
            blur: 14.0,
            spread: 0.0,
            color: colour.alpha(if p.k.dark { 0.3 } else { 0.18 }),
            inset: false,
        })
        .role("alert")
        .live_region(rune::LiveRegion::Polite)
        .hit()
        .on_press(move |s: &mut S| open(s, &id))
        .child(dot)
        .child(
            col()
                .key("question-lines")
                .gap(3.0)
                .grow(1.0)
                .min_w(0.0)
                .shrink()
                .child(
                    // Short enough that "asks" stays on the line.
                    text(tr_fill(
                        "{} asks",
                        &crate::assistant::store::shorten(&asking.title, 34),
                    ))
                    .key("question-who")
                    .truncate()
                    .font(p.k.ty.small_medium)
                    .color(colour),
                )
                .child(
                    text(asking.question.clone())
                        .key("question-text")
                        .wrap()
                        .max_lines(QUESTION_LINES)
                        .font(p.k.ty.ui_medium)
                        .color(p.text()),
                )
                .child(
                    row()
                        .key("question-foot")
                        .gap(6.0)
                        .center_y()
                        .child(
                            text(tr_fill("Hold {} to answer", answer_key))
                                .key("question-hint")
                                .font(p.k.ty.small)
                                .color(p.muted())
                                .grow(1.0)
                                .min_w(0.0),
                        )
                        .child(
                            Button::new(tr("Later"))
                                .variant(ButtonVariant::GhostMuted)
                                .control_size(ControlSize::Small)
                                .on_press(move |s: &mut S| defer(s, &later))
                                .build(p.k)
                                .key("later"),
                        ),
                ),
        )
        .child(
            IconButton::new(p.k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(tr("Hide question"))
                .on_press(move |s: &mut S| dismiss(s, &put_away))
                .build(p.k)
                .key("put-away"),
        )
}

/// The bubble a task keeps while the person has it, or while it waits out
/// a usage limit.
fn standing<S: 'static>(
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

/// A bubble popped about a task: what it says, with a press opening it in
/// Chats.
fn pop<S: 'static>(p: Paint, item: &Pop, actions: stack::Actions<S>) -> El {
    let colour = palette::task(item.colour, p.k.dark);
    let (lead, color, font): (El, _, _) = match item.kind {
        PopKind::New => (
            icon(&SPARKLES, 13.0).color(colour).a11y_hidden(),
            p.text(),
            p.k.ty.small_medium,
        ),
        PopKind::Step => (
            el().size(6.0, 6.0).radius(3.0).fill(colour).a11y_hidden(),
            p.muted(),
            p.k.ty.small,
        ),
        PopKind::Done => (
            icon(p.k.glyph.check, 14.0)
                .color(p.success())
                .label(tr("Done")),
            p.text(),
            p.k.ty.small_medium,
        ),
        PopKind::Failed => (
            icon(p.k.glyph.circle_x, 14.0)
                .color(p.danger())
                .label(tr("Failed")),
            p.danger(),
            p.k.ty.small_medium,
        ),
    };
    let open = actions.open;
    let id = item.task.clone();
    shell(p, format!("pop-{}", item.key))
        .pr(12.0)
        .role("status")
        .on_press(move |s: &mut S| open(s, &id))
        .child(lead.key("pop-lead"))
        .child(
            text(item.text.clone())
                .key("pop-text")
                .truncate()
                .font(font)
                .color(color)
                .min_w(0.0)
                .shrink(),
        )
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
