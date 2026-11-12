//! The task stack: Agent mode's tasks as small plates in a corner of the
//! screen, oldest on top, the newest nearest the corner. Each shows its
//! colour, its question, and what it's doing (or that it waits its turn),
//! with × to stop it; pressing a plate opens its conversation in Chats. A
//! finished task shows ✓ or ✗ with its answer's first line for a few
//! seconds (`board::FINISHED_LINGER`), then leaves.

use rune::{Cx, El, Transition, col, el, icon, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Spinner};

use super::super::controls::Paint;
use super::board::{Board, Phase, Task};
use super::palette;
use crate::app_settings::MeetingIndicatorCorner;
use crate::desktop::i18n::{tr, tr_fill};

/// The overlay: room for [`MAX_ROWS`] plates and the "more" line.
pub(crate) const SIZE: (f32, f32) = (352.0, 400.0);
/// Plates shown at most; older ones fold into "+N more".
pub(crate) const MAX_ROWS: usize = 6;
const ROW_H: f32 = 52.0;
const GAP: f32 = 6.0;
/// Room around the plates inside the overlay.
pub(crate) const INSET: f32 = 8.0;
/// From the corner of the display's visible area to the overlay's edge.
pub(crate) const MARGIN: f32 = 12.0;

/// What a press asks of the host.
pub(crate) struct Actions<S> {
    /// × on a task: stop it.
    pub(crate) stop: fn(&mut S, &str),
    /// A plate: open its conversation in Chats.
    pub(crate) open: fn(&mut S, &str),
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

/// The stack overlay's state: the board every task is on (the outlines
/// read it too), and whether the stack shows.
pub(crate) struct Stack {
    pub(crate) board: Board,
    pub(crate) shown: bool,
    /// The finished-task departure the overlay is woken for.
    pub(crate) armed: Option<std::time::Instant>,
    pub(crate) actions: Actions<Stack>,
}

pub(crate) fn view(stack: &Stack, cx: &mut Cx<Stack>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    el().key("stack-root")
        .w_full()
        .h_full()
        .padding(INSET)
        .hover_motion_within(rune::Anim::INSTANT, rune::Anim::INSTANT)
        .child(plates(p, &stack.board.stack(), stack.actions))
}

/// Which corner the stack takes: bottom right, unless the meeting
/// recording capsule lives there, then bottom left. The dictation HUD and
/// the notch indicator are at the top centre, the meeting offers top right.
pub(crate) fn corner(meeting: MeetingIndicatorCorner) -> MeetingIndicatorCorner {
    if meeting == MeetingIndicatorCorner::BottomRight {
        MeetingIndicatorCorner::BottomLeft
    } else {
        MeetingIndicatorCorner::BottomRight
    }
}

/// The overlay's rect (global points, y down) in `corner` of a display
/// whose visible area (below the menu bar, beside the Dock) is `visible`.
pub(crate) fn rect(visible: [f64; 4], corner: MeetingIndicatorCorner) -> [f32; 4] {
    let [x, y, w, h] = visible;
    let (sw, sh) = (f64::from(SIZE.0), f64::from(SIZE.1));
    let margin = f64::from(MARGIN - INSET);
    let left = if corner.is_right() {
        x + w - sw - margin
    } else {
        x + margin
    };
    let top = if corner.is_top() {
        y + margin
    } else {
        y + h - sh - margin
    };
    [left as f32, top as f32, SIZE.0, SIZE.1]
}

/// The plates, filling their parent from the bottom up.
pub(crate) fn plates<S: 'static>(p: Paint, tasks: &[Task], actions: Actions<S>) -> El {
    let hidden = tasks.len().saturating_sub(MAX_ROWS);
    let more = (hidden > 0).then(|| {
        text(tr_fill("+{} more", &hidden.to_string()))
            .key("more")
            .font(p.k.ty.small_medium)
            .color(p.muted())
            .pr(4.0)
    });
    col()
        .key("plates")
        .w_full()
        .h_full()
        .justify_end()
        .items_end()
        .gap(GAP)
        .role("list")
        .label(tr("Assistant tasks"))
        .children(more)
        .children(
            tasks[hidden..]
                .iter()
                .map(|task| plate(p, task, actions).transition(arrive(p))),
        )
}

/// A plate arrives from the corner and leaves fading.
fn arrive(p: Paint) -> Transition {
    Transition::asymmetric(
        Transition::offset(0.0, 12.0).combined(Transition::opacity()),
        Transition::opacity(),
    )
    .springs(rune::SpringPair::new(
        p.k.motion.overlay,
        p.k.motion.overlay,
    ))
}

fn plate<S: 'static>(p: Paint, task: &Task, actions: Actions<S>) -> El {
    let c = &p.k.c;
    let colour = palette::task(task.colour, p.k.dark);
    let live = task.phase.live();
    let dot = el()
        .key("dot")
        .size(10.0, 10.0)
        .radius(5.0)
        .fill(colour)
        .opacity(if task.phase == Phase::Queued {
            0.45
        } else {
            1.0
        })
        .a11y_hidden();
    let detail = task.detail().map(|line| {
        let color = match task.phase {
            Phase::Failed => p.danger(),
            Phase::Done => p.muted(),
            _ => p.faint(),
        };
        text(line)
            .key("detail")
            .truncate()
            .font(p.k.ty.small)
            .color(color)
    });
    let lines = col()
        .key("lines")
        .grow(1.0)
        .min_w(0.0)
        .child(
            text(task.title.clone())
                .key("title")
                .truncate()
                .font(p.k.ty.ui_medium)
                .color(if live { p.text() } else { p.muted() }),
        )
        .children(detail);
    let status = status(p, task, colour);
    let id = task.id.clone();
    let stop = live.then(|| {
        let id = id.clone();
        let stop = actions.stop;
        IconButton::new(p.k.glyph.close)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(tr("Stop task"))
            .on_press(move |s: &mut S| stop(s, &id))
            .build(p.k)
            .key("stop")
    });
    let open = actions.open;
    let body = row()
        .key(format!("task-{}", task.id))
        .w_full()
        .h(ROW_H)
        .pl(14.0)
        .pr(if live { 8.0 } else { 14.0 })
        .gap(10.0)
        .center_y()
        .radius(p.k.radius.md + 6.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .role("listitem")
        .label(task.title.clone())
        .hit()
        .on_press(move |s: &mut S| open(s, &id))
        .child(dot)
        .child(lines)
        .child(status)
        .children(stop);
    p.k.interactive(body)
}

/// Running: the activity spinner in the task's colour; queued, finished,
/// or stopped: a word or a mark.
fn status(p: Paint, task: &Task, colour: rune::Color) -> El {
    match task.phase {
        Phase::Running => Spinner::new()
            .size(14.0)
            .color(colour)
            .build(p.k)
            .key("spinner"),
        Phase::Queued => tag(p, tr("Queued"), "queued"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_keeps_clear_of_the_meeting_capsule() {
        use MeetingIndicatorCorner::*;
        assert_eq!(corner(TopRight), BottomRight);
        assert_eq!(corner(TopLeft), BottomRight);
        assert_eq!(corner(BottomLeft), BottomRight);
        assert_eq!(corner(BottomRight), BottomLeft);
    }

    #[test]
    fn the_stack_sits_in_the_visible_corner() {
        // A 1512 × 982 display with a 33-point menu bar and a 70-point Dock.
        let visible = [0.0, 33.0, 1512.0, 982.0 - 33.0 - 70.0];
        let [x, y, w, h] = rect(visible, MeetingIndicatorCorner::BottomRight);
        assert_eq!((w, h), SIZE);
        assert_eq!(x + w - INSET, 1512.0 - MARGIN);
        assert_eq!(y + h - INSET, 982.0 - 70.0 - MARGIN);
        let [x, ..] = rect(visible, MeetingIndicatorCorner::BottomLeft);
        assert_eq!(x + INSET, MARGIN);
    }
}
