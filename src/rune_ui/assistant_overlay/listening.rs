//! Conversation mode's listening: after an answer, while Hex listens
//! hands-free for a follow-up (`DictationIndicatorEvent::Listening`), a
//! calm glow breathes in the assistant's colours just past the HUD on the
//! display the person works on, with "Listening…" that fades after a
//! moment and a thin ring running down the seconds left. Speech starting
//! hands over to the HUD's question look; silence lets it go.

use std::time::{Duration, Instant};

use rune::{Cx, El, LineCap, Shape, Transition, canvas, el, row, text};
use rune_motion::{LoopSpec, Spring};

use super::super::controls::Paint;
use super::palette;
use crate::desktop::i18n::tr;

/// The overlay: room for the glow and its label in any language.
pub(crate) const SIZE: (f32, f32) = (220.0, 44.0);
/// The glow's ring, and the core inside it.
const RING: f32 = 26.0;
const RING_W: f32 = 2.0;
const CORE: f32 = 14.0;
/// One calm breath, in seconds.
const BREATH: f64 = 2.4;
/// How long "Listening…" shows before it fades.
pub(crate) const LABEL_FOR: Duration = Duration::from_millis(1_800);

/// The listening overlay's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Listening {
    /// Since when it listens, and for how long.
    pub(crate) since: Option<Instant>,
    pub(crate) seconds: f32,
    /// How much of the time is left (1 to 0), as last drawn.
    pub(crate) left: f32,
    /// "Listening…" still shows.
    pub(crate) label: bool,
}

impl Listening {
    /// Listening for `seconds` from `now`.
    pub(crate) fn start(now: Instant, seconds: f32) -> Self {
        Self {
            since: Some(now),
            seconds,
            left: 1.0,
            label: true,
        }
    }

    /// Moves the countdown and the label on to `now`; whether that changes
    /// what shows (the ring moves in steps of a hundredth).
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        let Some(since) = self.since else {
            return false;
        };
        let elapsed = now.saturating_duration_since(since);
        let left = if self.seconds > 0.0 {
            (1.0 - elapsed.as_secs_f32() / self.seconds).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let left = (left * 100.0).round() / 100.0;
        let label = elapsed < LABEL_FOR;
        let changed = left != self.left || label != self.label;
        self.left = left;
        self.label = label;
        changed
    }
}

pub(crate) fn view(state: &Listening, cx: &mut Cx<Listening>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    row()
        .key("listening-root")
        .w_full()
        .h_full()
        .center()
        .child(drawn(p, state, cx.system_reduced_motion()))
}

/// The glow with its countdown ring, and the label while it shows.
pub(crate) fn drawn(p: Paint, state: &Listening, still: bool) -> El {
    let (lead, trail) = palette::frame(p.k.dark);
    let core = el()
        .key("listening-core")
        .size(CORE, CORE)
        .radius(CORE / 2.0)
        .gradient(lead, trail)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 14.0,
            spread: 2.0,
            color: lead.alpha(0.55),
            inset: false,
        });
    let core = if still {
        core
    } else {
        core.looping(LoopSpec::opacity(BREATH, 0.0, 0.5, 1.0))
    };
    let glow = el()
        .key("listening-glow")
        .size(RING, RING)
        .center()
        .child(
            canvas(ring(state.left, lead, p.k.c.border_base))
                .key("listening-ring")
                .absolute(0.0, 0.0)
                .size(RING, RING),
        )
        .child(core)
        .a11y_hidden();
    let label = state.label.then(|| {
        text(tr("Listening…"))
            .key("listening-label")
            .font(p.k.ty.small_medium)
            .color(p.muted())
            .transition(Transition::opacity().springs(rune::SpringPair::new(
                Spring::new(0.3, 0.0),
                Spring::new(0.9, 0.0),
            )))
    });
    let c = &p.k.c;
    row()
        .key("listening")
        .h(RING + 8.0)
        .pl(5.0)
        .pr(if state.label { 12.0 } else { 5.0 })
        .gap(8.0)
        .center_y()
        .radius((RING + 8.0) / 2.0)
        .fill(c.bg_base.alpha(0.9))
        .border(p.hair, c.border_base)
        .role("status")
        .label(tr("Listening…"))
        .animate_layout()
        .child(glow)
        .children(label)
}

/// A ring `RING` points across: a faint track, and what's left of the time
/// in `colour`, running down clockwise from the top.
fn ring(left: f32, colour: rune::Color, track: rune::Color) -> Shape {
    let r = (RING - RING_W) / 2.0;
    let c = RING / 2.0;
    let point = |t: f32| {
        let a = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        [c + r * a.cos(), c + r * a.sin()]
    };
    const SEGMENTS: usize = 64;
    let track_points: Vec<[f32; 2]> = (0..SEGMENTS)
        .map(|i| point(i as f32 / SEGMENTS as f32))
        .collect();
    let shape = Shape::new().stroke_with(track_points, true, RING_W, LineCap::Round, track);
    if left <= 0.0 {
        return shape;
    }
    let steps = ((SEGMENTS as f32 * left).ceil() as usize).max(2);
    let arc: Vec<[f32; 2]> = (0..=steps)
        .map(|i| point(1.0 - left + left * i as f32 / steps as f32))
        .collect();
    shape.stroke_with(arc, false, RING_W, LineCap::Round, colour.alpha(0.85))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_countdown_runs_down_and_the_label_fades_after_a_moment() {
        let t0 = Instant::now();
        let mut state = Listening::start(t0, 8.0);
        assert!(!state.tick(t0), "nothing moved yet");
        assert!(state.tick(t0 + Duration::from_secs(2)));
        assert_eq!(state.left, 0.75);
        assert!(!state.label, "Listening… has faded");
        assert!(state.tick(t0 + Duration::from_secs(9)));
        assert_eq!(state.left, 0.0);
        let mut off = Listening::start(t0, 0.0);
        off.tick(t0);
        assert_eq!(off.left, 0.0, "no time to count down");
    }
}
