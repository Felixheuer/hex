//! The screen frame: while Agent mode is on, a soft glow breathes around
//! the edge of every display, so it's plain from across the room that
//! agents are at work. Two hues glow inward from the edges (one from the
//! top and left, one from the bottom and right) over a hairline rim.
//!
//! The drawing is static: on macOS the panel's layer breathes in Core
//! Animation (`macos::breathe`), so a frame that stays up for hours costs
//! no redraws. The playground, which has no panel, breathes it here.

use rune::{Cx, El, Shadow, el};

use super::palette;

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
/// One breath, in and out, in seconds.
pub(crate) const BREATH: f64 = 4.2;
/// The frame's opacity at the bottom of a breath.
pub(crate) const BREATH_LOW: f32 = 0.55;

/// A frame overlay's state: one per display.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Frame {
    /// The display it covers (`NSScreenNumber`), while it shows.
    pub(crate) display: Option<u32>,
}

pub(crate) fn view(_frame: &Frame, cx: &mut Cx<Frame>) -> El {
    glow(cx.system_dark(), false)
}

/// The frame, filling its parent. `breathe` loops its opacity on Rune's
/// clock (the playground); reduced motion holds it still.
pub(crate) fn glow(dark: bool, breathe: bool) -> El {
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
    let frame = el()
        .key("frame")
        .w_full()
        .h_full()
        .radius(CORNER)
        .box_shadow(inset(LEAN, LEAN, HALO, lead.alpha(halo)))
        .box_shadow(inset(-LEAN, -LEAN, HALO, trail.alpha(halo)))
        .box_shadow(inset(LEAN / 3.0, LEAN / 3.0, GLOW, lead.alpha(inner)))
        .box_shadow(inset(-LEAN / 3.0, -LEAN / 3.0, GLOW, trail.alpha(inner)))
        .border_gradient(RIM, lead.alpha(rim), trail.alpha(rim))
        .a11y_hidden();
    if breathe {
        frame.looping(rune_motion::LoopSpec::opacity(BREATH, 0.0, BREATH_LOW, 1.0))
    } else {
        frame
    }
}
