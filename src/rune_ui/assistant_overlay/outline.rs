//! A window outline: a thin line in its task's colour around a window the
//! task works in, with a soft glow and a small label naming the task. Each
//! outline is its own click-through overlay, sized to its window plus a
//! margin and moved with it, so a moving window costs a move, not a redraw.
//!
//! Its look follows the task: dashed with "You have it" while the person
//! has taken it over, dimmed with when it resumes while it waits out a
//! usage limit, and a soft line without a label while a task moves the real
//! pointer there ("Hands off").

use rune::{Cx, El, Shadow, el, row, text};

use super::super::controls::Paint;
use super::board::Look;
use super::{palette, when};
use crate::desktop::i18n::{tr, tr_fill};

/// Room around the window for the glow, in points.
pub(crate) const PAD: f32 = 10.0;
/// The label's height.
pub(crate) const LABEL_H: f32 = 20.0;
/// Between the label and the outline.
const LABEL_GAP: f32 = 4.0;
/// The label's widest.
const LABEL_MAX_W: f32 = 260.0;
/// macOS windows' corner radius (Sonoma's; Tahoe's are rounder, and the
/// glow covers the difference).
const WINDOW_CORNER: f32 = 10.0;
const LINE: f32 = 2.0;
/// A waiting task's outline, dimmed.
const WAITING_OPACITY: f32 = 0.5;

/// Where the label sits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum LabelSide {
    /// Above the window's top-left corner, outside it.
    #[default]
    Above,
    /// Inside its bottom-left corner: no room above (a window at the top
    /// of its display).
    Inside,
}

/// An outline overlay's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Outline {
    pub(crate) colour: u8,
    pub(crate) title: String,
    /// The window's size in points.
    pub(crate) size: (f32, f32),
    pub(crate) label: LabelSide,
    pub(crate) look: Look,
}

/// The overlay's rect in global points (x, y, width, height) for a window
/// at `window` (global points, y down), and where its label goes: above
/// unless that would leave `top`, the top of the display the window is on.
pub(crate) fn rect(window: (f64, f64, f64, f64), top: f64) -> ([f32; 4], LabelSide) {
    let (x, y, w, h) = window;
    let above = f64::from(LABEL_H + LABEL_GAP);
    let label = if y - above >= top {
        LabelSide::Above
    } else {
        LabelSide::Inside
    };
    let room = match label {
        LabelSide::Above => LABEL_H + LABEL_GAP,
        LabelSide::Inside => 0.0,
    };
    let pad = f64::from(PAD);
    (
        [
            (x - pad) as f32,
            (y - pad - f64::from(room)) as f32,
            (w + 2.0 * pad) as f32,
            (h + 2.0 * pad) as f32 + room,
        ],
        label,
    )
}

pub(crate) fn view(outline: &Outline, cx: &mut Cx<Outline>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    el().key("outline-root")
        .w_full()
        .h_full()
        .child(drawn(p, outline, 0.0, 0.0))
}

/// The outline with its label, its window's top-left at `(x, y)` plus the
/// margin the overlay leaves.
pub(crate) fn drawn(p: Paint, outline: &Outline, x: f32, y: f32) -> El {
    let colour = palette::task(outline.colour, p.k.dark);
    let ink = palette::ink(colour, p.k);
    let (w, h) = outline.size;
    let room = match outline.label {
        LabelSide::Above => LABEL_H + LABEL_GAP,
        LabelSide::Inside => 0.0,
    };
    let (left, top) = (x + PAD, y + PAD + room);
    let line = el()
        .key("line")
        .absolute(left, top)
        .size(w, h)
        .radius(WINDOW_CORNER)
        .a11y_hidden();
    let line = match outline.look {
        Look::HandsOff => {
            line.border(LINE * 0.75, colour.alpha(0.7))
                .outline(6.0, 0.0, colour.alpha(0.14))
        }
        Look::Paused => {
            line.border(LINE, colour)
                .border_dash(10.0, 6.0)
                .outline(4.0, 0.0, colour.alpha(0.16))
        }
        Look::Working | Look::Waiting { .. } => line
            .border(LINE, colour)
            // A soft ring outside the line; a drop shadow would also paint
            // under the (clear) element and tint the window.
            .outline(4.0, 0.0, colour.alpha(0.28))
            .box_shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 10.0,
                spread: 0.0,
                color: colour.alpha(0.22),
                inset: true,
            }),
    };
    let label_top = match outline.label {
        LabelSide::Above => y + PAD,
        LabelSide::Inside => top + h - LABEL_H - 8.0,
    };
    let label_left = match outline.label {
        LabelSide::Above => left + 2.0,
        LabelSide::Inside => left + 8.0,
    };
    let words = match outline.look {
        Look::Working => Some(outline.title.clone()),
        Look::Paused => Some(tr("You have it").to_owned()),
        Look::Waiting { until_ms } => Some(tr_fill(
            "Waiting for limit · resumes {}",
            &when::clock(until_ms),
        )),
        Look::HandsOff => None,
    };
    let label = words.map(|words| {
        row()
            .key("label")
            .absolute(label_left, label_top)
            .h(LABEL_H)
            .max_w(LABEL_MAX_W.min(w - 16.0).max(40.0))
            .px(8.0)
            .center_y()
            .radius(LABEL_H / 2.0)
            .fill(colour)
            .child(
                text(words)
                    .key("label-text")
                    .truncate()
                    .font(p.k.ty.small_medium)
                    .color(ink),
            )
    });
    let drawn = el()
        .key("outline")
        .w_full()
        .h_full()
        .child(line)
        .children(label);
    if matches!(outline.look, Look::Waiting { .. }) {
        drawn.opacity(WAITING_OPACITY)
    } else {
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_goes_above_unless_the_window_meets_the_top_of_its_display() {
        let (rect_above, side) = rect((100.0, 200.0, 800.0, 600.0), 0.0);
        assert_eq!(side, LabelSide::Above);
        assert_eq!(
            rect_above,
            [
                100.0 - PAD,
                200.0 - PAD - LABEL_H - LABEL_GAP,
                800.0 + 2.0 * PAD,
                600.0 + 2.0 * PAD + LABEL_H + LABEL_GAP,
            ]
        );
        // Just under the menu bar of a display above the primary one.
        let (rect_inside, side) = rect((0.0, -1060.0, 1200.0, 900.0), -1080.0);
        assert_eq!(side, LabelSide::Inside);
        assert_eq!(
            rect_inside,
            [-PAD, -1060.0 - PAD, 1200.0 + 2.0 * PAD, 900.0 + 2.0 * PAD]
        );
    }
}
