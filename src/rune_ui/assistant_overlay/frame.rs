//! The screen frame: while Background work runs, a soft glow breathes
//! around the edge of every display, so it's plain from across the room
//! that agents are at work. Two hues glow inward from the edges (one from
//! the top and left, one from the bottom and right) over a hairline rim.
//!
//! Where two displays touch, the glow moves off the shared stretch so the
//! frame runs on around the whole desk instead of doubling at the seam;
//! the parts of an edge that overhang the neighbour glow on their own
//! (`screens::seams`).
//!
//! The drawing is static: on macOS the panel's layer breathes in Core
//! Animation (`host::watcher`), so a frame that stays up for hours costs no
//! redraws. The playground, which has no panel, breathes it here.

use rune::{Color, Cx, El, Shadow, el};

use super::palette;
use super::screens::{Edge, Seams, Strip};

/// The display's corner radius the glow follows (built-in displays round
/// their corners; on square ones the corners stay clear).
const CORNER: f32 = 10.0;
/// How far the wide halo reaches in from the edge.
const HALO: f32 = 110.0;
/// How far the bright inner glow reaches.
const GLOW: f32 = 26.0;
/// How far each hue leans toward its own edges.
const LEAN: f32 = 14.0;
/// The rim's width.
const RIM: f32 = 1.5;
/// How far the frame's own drawing moves past a joined edge, so none of
/// its glow is left on the panel there.
const PAST: f32 = HALO + 2.0 * LEAN;
/// How far an open stretch's glow reaches in, and how it fades at a seam.
const STRIP_DEPTH: f32 = 80.0;
const STRIP_FADE: f32 = 48.0;
/// One breath, in and out, in seconds.
pub(crate) const BREATH: f64 = 4.2;
/// The frame's opacity at the bottom of a breath.
pub(crate) const BREATH_LOW: f32 = 0.55;

/// A frame overlay's state: one per display.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Frame {
    /// The display it covers (`NSScreenNumber`), while it shows.
    pub(crate) display: Option<u32>,
    /// The display's size in points.
    pub(crate) size: (f32, f32),
    /// Where it meets other displays.
    pub(crate) seams: Seams,
}

pub(crate) fn view(frame: &Frame, cx: &mut Cx<Frame>) -> El {
    glow_joined(cx.system_dark(), false, frame.size, &frame.seams)
}

/// The frame on a display alone, filling its parent. `breathe` loops its
/// opacity on Rune's clock (the playground); reduced motion holds it still.
pub(crate) fn glow(dark: bool, breathe: bool) -> El {
    breathing(ring(dark), breathe)
}

/// The frame on a display of `size` meeting others along `seams`.
pub(crate) fn glow_joined(dark: bool, breathe: bool, size: (f32, f32), seams: &Seams) -> El {
    if !seams.joined.iter().any(|joined| *joined) {
        return glow(dark, breathe);
    }
    let past = |edge| if seams.joined(edge) { PAST } else { 0.0 };
    let (left, top) = (past(Edge::Left), past(Edge::Top));
    let ring = ring(dark).absolute(-left, -top).size(
        size.0 + left + past(Edge::Right),
        size.1 + top + past(Edge::Bottom),
    );
    let (lead, trail) = palette::frame(dark);
    let strips = seams.open.iter().map(|strip| {
        open_strip(
            *strip,
            size,
            if matches!(strip.edge, Edge::Top | Edge::Left) {
                lead
            } else {
                trail
            },
            dark,
        )
    });
    let joined = el()
        .key("frame-joined")
        .w_full()
        .h_full()
        .clip_rounded(0.0)
        .a11y_hidden()
        .child(ring)
        .children(strips);
    breathing(joined, breathe)
}

fn breathing(frame: El, breathe: bool) -> El {
    if breathe {
        frame.looping(rune_motion::LoopSpec::opacity(BREATH, 0.0, BREATH_LOW, 1.0))
    } else {
        frame
    }
}

/// The glow around a whole display, filling its parent.
fn ring(dark: bool) -> El {
    let (lead, trail) = palette::frame(dark);
    // Brighter over dark desktops reads as light; over light ones the glow
    // needs more colour to show at all.
    let (halo, inner, rim) = if dark {
        (0.34, 0.62, 0.9)
    } else {
        (0.3, 0.55, 0.85)
    };
    let inset = |x: f32, y: f32, blur: f32, color| Shadow {
        x,
        y,
        blur,
        spread: 0.0,
        color,
        inset: true,
    };
    el().key("frame")
        .w_full()
        .h_full()
        .radius(CORNER)
        .box_shadow(inset(LEAN, LEAN, HALO, lead.alpha(halo)))
        .box_shadow(inset(-LEAN, -LEAN, HALO, trail.alpha(halo)))
        .box_shadow(inset(LEAN / 3.0, LEAN / 3.0, GLOW, lead.alpha(inner)))
        .box_shadow(inset(-LEAN / 3.0, -LEAN / 3.0, GLOW, trail.alpha(inner)))
        .border_gradient(RIM, lead.alpha(rim), trail.alpha(rim))
        .a11y_hidden()
}

/// The glow along an open stretch of a joined edge: a gradient fading in
/// from the edge, with the rim, softened where it meets the seam.
fn open_strip(strip: Strip, (w, h): (f32, f32), colour: Color, dark: bool) -> El {
    let (strong, rim) = if dark { (0.55, 0.9) } else { (0.5, 0.85) };
    let len = strip.to - strip.from;
    let stops = [
        (0.0, colour.alpha(strong)),
        (0.3, colour.alpha(strong * 0.4)),
        (1.0, colour.alpha(0.0)),
    ];
    // Where it starts, its size, and which way it fades in.
    let (x, y, sw, sh, angle) = match strip.edge {
        Edge::Top => (strip.from, 0.0, len, STRIP_DEPTH, std::f32::consts::PI),
        Edge::Bottom => (strip.from, h - STRIP_DEPTH, len, STRIP_DEPTH, 0.0),
        Edge::Left => (
            0.0,
            strip.from,
            STRIP_DEPTH,
            len,
            std::f32::consts::FRAC_PI_2,
        ),
        Edge::Right => (
            w - STRIP_DEPTH,
            strip.from,
            STRIP_DEPTH,
            len,
            3.0 * std::f32::consts::FRAC_PI_2,
        ),
    };
    let full = match strip.edge {
        Edge::Top | Edge::Bottom => w,
        Edge::Left | Edge::Right => h,
    };
    // Soften the ends that stop at a seam, not at the display's corner.
    let fade_start = if strip.from > 0.5 { STRIP_FADE } else { 0.0 };
    let fade_end = if strip.to < full - 0.5 {
        STRIP_FADE
    } else {
        0.0
    };
    let (fl, ft, fr, fb) = match strip.edge {
        Edge::Top | Edge::Bottom => (fade_start, 0.0, fade_end, 0.0),
        Edge::Left | Edge::Right => (0.0, fade_start, 0.0, fade_end),
    };
    let rim_line = match strip.edge {
        Edge::Top => [0.0, 0.0, sw, RIM],
        Edge::Bottom => [0.0, sh - RIM, sw, RIM],
        Edge::Left => [0.0, 0.0, RIM, sh],
        Edge::Right => [sw - RIM, 0.0, RIM, sh],
    };
    el().key(format!("frame-open-{:?}-{}", strip.edge, strip.from as i32))
        .absolute(x, y)
        .size(sw, sh)
        .linear_gradient(angle, stops)
        .clip_fade(fl, ft, fr, fb)
        .child(
            el().absolute(rim_line[0], rim_line[1])
                .size(rim_line[2], rim_line[3])
                .fill(colour.alpha(rim)),
        )
        .a11y_hidden()
}
