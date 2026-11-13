//! A window outline, tier 2 of the three frames (`docs/ux/redesign.md`
//! §3.5): a 2-point line in its task's colour (the agent's hue for a Here
//! task) with a soft ring around a window the task works in **that the
//! person can see**, and a chip above its top-left naming it: dot · title ·
//! what it does there (· 2/5). Parked windows get none (their minis' rings
//! say it, `pictures::ring`). Each outline is its own click-through
//! overlay, sized to its window plus a margin and moved with it, so a
//! moving window costs a move, not a redraw.
//!
//! Its look follows the task, and the chip keeps the title in every look:
//! dashed with "you have it" while the person has taken it over, dimmed
//! with when it resumes while it waits out a usage limit, and solid while
//! a task moves the real pointer there ("Hands off", under the desk frame).
//! Without room above (a window at the top of its display) the chip sits
//! inside the window's top-left corner.

use rune::{Cx, El, Shadow, el, row, text};

use super::super::controls::Paint;
use super::board::Look;
use super::{look, palette, when};
use crate::desktop::i18n::{tr, tr_fill};

/// Room around the window for the glow, in points.
pub(crate) const PAD: f32 = 10.0;
/// The label's height.
pub(crate) const LABEL_H: f32 = 22.0;
/// Between the label and the outline.
const LABEL_GAP: f32 = 4.0;
/// The label's widest.
const LABEL_MAX_W: f32 = 380.0;
/// The title's widest inside it, so what it does keeps room.
const TITLE_MAX_W: f32 = 220.0;
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
    /// Inside its top-left corner: no room above (a window at the top of
    /// its display).
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
    /// Its plan's steps done and in all, shown after its title.
    pub(crate) progress: Option<(usize, usize)>,
    /// What it does there now, shown after its title.
    pub(crate) step: Option<String>,
    /// A Here task's window: drawn in the agent's hue, not a task colour.
    pub(crate) here: bool,
    /// Where the whole outline starts in this panel: (0, 0), or less for a
    /// piece of one spanning displays.
    pub(crate) origin: (f32, f32),
    /// The label's widest here, when a card beside it leaves it less
    /// than its own (`place::label_room`): it shortens rather than hide
    /// under the card.
    pub(crate) room: Option<f32>,
}

/// The label's least width: its dot and the start of the title.
pub(crate) const LABEL_LEAST_W: f32 = 120.0;

/// About how wide `outline`'s label draws for a window `width` wide.
pub(crate) fn label_width(outline: &Outline, width: f32) -> f32 {
    let title = (outline.title.chars().count() as f32 * 7.0).min(TITLE_MAX_W);
    let state = state_words(outline).map_or(0.0, |state| 12.0 + state.chars().count() as f32 * 6.2);
    (8.0 + 7.0 + 6.0 + title + state + 9.0).min(LABEL_MAX_W.min(width - 16.0).max(40.0))
}

/// Where the label of a window at `window` (global points) draws, whole
/// and at its least, `wide` its natural width (`label_width`), `top` the
/// top of the display (as [`rect`]).
pub(crate) fn label_rects(
    window: (f64, f64, f64, f64),
    top: f64,
    wide: f32,
) -> ([f64; 4], [f64; 4]) {
    let (x, y, _, _) = window;
    let (lx, ly) = match rect(window, top).1 {
        LabelSide::Above => (x + 2.0, y - f64::from(LABEL_GAP + LABEL_H)),
        LabelSide::Inside => (x + 8.0, y + 8.0),
    };
    let h = f64::from(LABEL_H);
    (
        [lx, ly, f64::from(wide), h],
        [lx, ly, f64::from(LABEL_LEAST_W.min(wide)), h],
    )
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
    el().key("outline-root").w_full().h_full().child(drawn(
        p,
        outline,
        outline.origin.0,
        outline.origin.1,
    ))
}

/// The outline with its label, its window's top-left at `(x, y)` plus the
/// margin the overlay leaves.
pub(crate) fn drawn(p: Paint, outline: &Outline, x: f32, y: f32) -> El {
    let colour = if outline.here {
        palette::agent(p.k.dark)
    } else {
        palette::task(outline.colour, p.k.dark)
    };
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
        Look::Paused => {
            line.border(LINE, colour)
                .border_dash(10.0, 6.0)
                .outline(4.0, 0.0, colour.alpha(0.16))
        }
        Look::Working | Look::HandsOff | Look::Waiting { .. } => line
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
    let (label_left, label_top) = match outline.label {
        LabelSide::Above => (left + 2.0, y + PAD),
        LabelSide::Inside => (left + 8.0, top + 8.0),
    };
    let label = chip(p, outline, colour)
        .absolute(label_left, label_top)
        .max_w(
            LABEL_MAX_W
                .min(w - 16.0)
                .min(outline.room.unwrap_or(f32::MAX))
                .max(40.0),
        );
    let drawn = el()
        .key("outline")
        .w_full()
        .h_full()
        .child(line)
        .child(label);
    if matches!(outline.look, Look::Waiting { .. }) {
        drawn.opacity(WAITING_OPACITY)
    } else {
        drawn
    }
}

/// What the chip says after the title in `outline`'s look: the step and
/// progress while it works, "you have it" while the person has it, when
/// it resumes while it waits.
pub(crate) fn state_words(outline: &Outline) -> Option<String> {
    match outline.look {
        Look::Working | Look::HandsOff => {
            let mut parts: Vec<String> = outline.step.iter().cloned().collect();
            if let Some((done, total)) = outline.progress {
                parts.push(format!("{done}/{total}"));
            }
            (!parts.is_empty()).then(|| parts.join(" · "))
        }
        Look::Paused => Some(tr("you have it").to_owned()),
        Look::Waiting { until_ms } => Some(tr_fill("resumes {}", &when::clock(until_ms))),
    }
}

/// The chip: a dot in the outline's colour, the task's title, and what it
/// does now, on the card material so it reads over any window.
fn chip(p: Paint, outline: &Outline, colour: rune::Color) -> El {
    let c = &p.k.c;
    let state = state_words(outline);
    let spoken = match &state {
        Some(state) => format!("{}, {state}", outline.title),
        None => outline.title.clone(),
    };
    row()
        .key("label")
        .h(LABEL_H)
        .pl(8.0)
        .pr(9.0)
        .gap(6.0)
        .center_y()
        .radius(LABEL_H / 2.0)
        .fill(c.bg_base.alpha(1.0))
        .border(1.0, colour.alpha(0.7))
        .box_shadow(Shadow {
            x: 0.0,
            y: 2.0,
            blur: 8.0,
            spread: 0.0,
            color: rune::Color::BLACK.alpha(if p.k.dark { 0.4 } else { 0.14 }),
            inset: false,
        })
        .label(spoken)
        .child(look::dot(colour, 7.0))
        .child(
            text(outline.title.clone())
                .key("label-text")
                .truncate()
                .font(look::label(p))
                .color(p.text())
                .max_w(TITLE_MAX_W)
                .a11y_hidden(),
        )
        .children(state.map(|state| {
            text(format!("· {state}"))
                .key("label-state")
                .truncate()
                .font(look::meta(p))
                .color(p.muted())
                .min_w(0.0)
                .a11y_hidden()
        }))
}

/// How much two window rects (x, y, width, height) overlap: the shared
/// area over the smaller one's, 0 to 1.
fn shared(a: [f64; 4], b: [f64; 4]) -> f64 {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    if right <= x || bottom <= y {
        return 0.0;
    }
    let smaller = (a[2] * a[3]).min(b[2] * b[3]);
    if smaller <= 0.0 {
        return 0.0;
    }
    (right - x) * (bottom - y) / smaller
}

/// Whether `window` fills `display`: its edges within a few points of the
/// display's (the top may leave room for the menu bar).
fn fills(window: [f64; 4], display: [f64; 4]) -> bool {
    const SIDE: f64 = 16.0;
    const TOP: f64 = 48.0;
    window[0] <= display[0] + SIDE
        && window[1] <= display[1] + TOP
        && window[0] + window[2] >= display[0] + display[2] - SIDE
        && window[1] + window[3] >= display[1] + display[3] - SIDE
}

/// Whether an outline around `window` would only double what's drawn: a
/// window already outlined (`drawn`: the same window named twice, or two
/// tasks in one window) gets one outline, the first's; and while the screen
/// frame shows, a window filling its display gets none, since the frame
/// already runs along its edges.
pub(crate) fn redundant(
    window: [f64; 4],
    drawn: &[[f64; 4]],
    displays: &[[f64; 4]],
    frame_on: bool,
) -> bool {
    const SAME: f64 = 0.9;
    drawn.iter().any(|other| shared(window, *other) >= SAME)
        || frame_on && displays.iter().any(|display| fills(window, *display))
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

    #[test]
    fn one_outline_per_window_and_none_along_the_frame() {
        let display = [0.0, 0.0, 1800.0, 1169.0];
        let mail = [100.0, 80.0, 900.0, 700.0];
        // The same window named twice, or a second task in it: one outline.
        assert!(redundant(mail, &[mail], &[display], false));
        assert!(redundant(
            [104.0, 82.0, 896.0, 698.0],
            &[mail],
            &[display],
            false
        ));
        // Two windows side by side: both outlined.
        assert!(!redundant(
            [1000.0, 80.0, 700.0, 700.0],
            &[mail],
            &[display],
            false
        ));
        // A window filling the display doubles the screen frame's edge…
        let full = [0.0, 37.0, 1800.0, 1132.0];
        assert!(redundant(full, &[], &[display], true));
        // …but is outlined while no frame shows.
        assert!(!redundant(full, &[], &[display], false));
        assert!(!redundant(mail, &[], &[display], true));
    }
}
