//! The rail: Background jobs' quiet presence, top right under the menu bar
//! (or wherever the person dragged it on that display, [`super::place`]).
//! Its head is the orb's glowing core, breathing, and in words how many
//! jobs run and how many questions wait ("3 jobs · 1 needs you").
//!
//! Under it, one calm column, nearest the head first, so nothing overlaps
//! or jumps (`docs/ux/redesign.md` §3.6: head + 1 question + 3 job rows +
//! 1 result):
//!
//! - The question to answer now (the inbox's active one,
//!   `Event::QuestionsChanged`): who asks, the question, and how to
//!   answer, by voice ("Hold ⌘ and say it") or typed right there
//!   ([`super::ask`]); Later puts it behind the others.
//! - A task the person took over keeps "You have it" with Hand back; one
//!   waiting out a usage limit keeps "Waiting for limit · resumes 14:05".
//! - The running jobs as rows with where they work, what they do now, and
//!   live pictures, the rest summed up in one "N more" line
//!   ([`super::jobs`]).
//! - A finished job's result ([`RAIL_RESULTS`]), which fades, with what it
//!   names to open and Try again on a failure ([`super::results`]).
//!
//! A job with a plan wears a progress ring around its dot and a plan bar
//! on its row. After the last job ends the rail stays, quieter, while it
//! can recall a result.
//!
//! The full list ([`super::stack`]) unfolds from the orb on a press, a
//! double-tap of the Action shortcut, or a press on a job's row, and folds
//! back on ×, Escape, a click outside, or another press. Everything grows
//! from the orb's side of the panel with springs, and nothing but the orb
//! and its bubbles takes clicks.

use super::look;
use rune::{Cx, El, LineCap, Shape, Transition, Vec2, canvas, col, el, icon, row, text};
use rune_motion::{LoopSpec, Spring};

mod bubbles;
mod cluster;

use super::super::controls::Paint;
use super::ask::{self, Reply};
use super::board::{Board, Phase, Task};
use super::minis::{self, Desk};
use super::pops::MAX_POPS;
use super::results::{self, Outcome};
use super::{frame, jobs, palette, stack, wall, when};
use crate::desktop::i18n::{tr, tr_fill};
use bubbles::{all_waiting, anchor, pop_in, standing_bubble, unfold};
use cluster::orb;
pub(crate) use cluster::{dot, marker};

/// The panel at its tallest: room for the list unfolded, or the job rows
/// with their pictures. On a display with less room past the orb the host
/// makes it shorter ([`super::place::fitted`]) and says how tall
/// ([`Orb::room`]); what unfolds takes that height and scrolls inside it.
pub(crate) const SIZE: (f32, f32) = (400.0, 860.0);
/// Room around everything inside the panel.
pub(crate) const INSET: f32 = 10.0;
/// The rail's head's height (it's as wide as its words need).
pub(crate) const ORB_H: f32 = 36.0;
/// The glowing core's size.
const CORE: f32 = 22.0;
const GAP: f32 = 8.0;
/// A bubble's widest.
const BUBBLE_MAX_W: f32 = jobs::CARD_W;
const BUBBLE_H: f32 = 36.0;
/// A job row's height with its strip of pictures: its head, line, the
/// pictures with their names, and its plan bar.
const ROW_MIN: f32 = 150.0;
/// Results in the rail at once (`docs/ux/redesign.md` §3.6): the newest,
/// a failure before a success; the list recalls the rest.
pub(crate) const RAIL_RESULTS: usize = 1;
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
            jobs: jobs::LineActions {
                wall: |_, _| {},
                all: |_| {},
            },
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
    /// The panel's height as placed on its display (at most `SIZE.1`).
    pub(crate) room: f32,
    /// The person's desk as last read.
    pub(crate) desk: Desk,
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
            room: SIZE.1,
            desk: Desk::default(),
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
    /// The panel's height, when it isn't `SIZE.1`.
    pub(crate) room: Option<f32>,
    /// The person's desk: its pictures follow the job rows.
    pub(crate) desk: Desk,
}

impl Look {
    /// The height everything at the orb may take, inside the panel's inset.
    pub(crate) fn inner_h(&self) -> f32 {
        self.room.unwrap_or(SIZE.1).min(SIZE.1) - 2.0 * INSET
    }
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
        room: Some(orb.room),
        desk: orb.desk.clone(),
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
/// one is, else the newest of the last [`MAX_POPS`] (a failure first), at
/// most [`RAIL_RESULTS`], each with its result.
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
    // A failure first (it waits for Try again), else the newest.
    if let Some(at) = items.iter().position(|(_, outcome)| outcome.failed) {
        items[..=at].rotate_right(1);
    }
    items.truncate(RAIL_RESULTS);
    items
}

/// The orb and what unfolds from it, filling its parent from the orb's
/// corner.
pub(crate) fn content<S: 'static>(p: Paint, board: &Board, look: &Look, actions: Actions<S>) -> El {
    let tasks = board.stack();
    let done = board.results();
    let grow = look.grow;
    let mut parts: Vec<El> = Vec::new();
    let desk = !look.desk.is_empty();
    let inner_h = look.inner_h();
    if !tasks.is_empty() || !board.pops.items.is_empty() || !done.is_empty() || desk {
        // The orb, and beside it "All N live": every live picture on the
        // wall, one press away.
        let mut head = vec![orb(
            p,
            &tasks,
            done.len(),
            board.open_questions(),
            look,
            actions,
        )];
        head.extend(wall::all_live_chip(
            p,
            wall::count_all(board, &look.desk),
            actions.jobs.all,
        ));
        if !grow.right {
            head.reverse();
        }
        parts.push(row().key("orb-head").gap(GAP).center_y().children(head));
    }
    if look.open && (!tasks.is_empty() || !done.is_empty()) {
        let max_h = inner_h - ORB_H - GAP;
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
        while results.len() > 1 && inner_h - fixed - results_room(&results) < rows_need {
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
            max_h: (inner_h - taken).max(jobs::LINE_H + 2.0 * GAP),
            appear: pop_in(grow),
        };
        // The job rows and the person's desk (what quick turns opened,
        // live, until let go) in one column that scrolls inside the panel.
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
                minis::view(p, &look.desk, actions.list),
                actions.list,
                actions.jobs,
            )
            .map(|rows| rows.transition(pop_in(grow))),
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
    // Never taller than the panel: the job rows give way and scroll.
    let cluster = col()
        .key("cluster")
        .max_h(inner_h)
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
