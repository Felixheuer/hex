//! The desk frame, tier 1 of the three frames (`docs/ux/redesign.md` §3.5):
//! a still glow around the edge of every display, shown exactly while the
//! agent has the person's controls: a task moves the real pointer or keys
//! ("Hands off"), or the guard brought one of a task's windows in front for
//! a moment (a borrow). Work in parked windows, hidden browsers, or files
//! never lights it (`assistant::view::Stream::frame_until`). Two hues glow
//! inward from the edges (one from the top and left, one from the bottom
//! and right) over a hairline rim; it doesn't breathe: the agent is in
//! front, and nothing should pulse at the person meanwhile.
//!
//! It always comes with its **tab**, a black capsule hanging under the
//! island on the display it's about: "Agent-Desk is using Numbers for a
//! moment", then why, "Esc to stop", and the countdown from the borrow's
//! budget ("back in 2 s"). When the front goes back, the glow goes and the
//! tab says "Back to you · Mail" for a moment ([`Tab::Back`]).
//!
//! Where two displays touch, the glow moves off the shared stretch so the
//! frame runs on around the whole desk instead of doubling at the seam;
//! the parts of an edge that overhang the neighbour glow on their own
//! (`screens::seams`).

use std::time::Instant;

use rune::{Color, Cx, El, Shadow, col, el, icon, row, text};

use super::super::controls::Paint;
use super::look;
use super::palette;
use super::screens::{Edge, Seams, Strip};
use crate::assistant::TaskWindow;
use crate::assistant::view::{BACK_FOR, Stream};
use crate::desktop::i18n::{tr, tr_fill};

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
/// One breath of the orb's cluster, in and out, in seconds.
pub(crate) const BREATH: f64 = 4.2;
/// From the display's top to the tab: past the menu bar and the island
/// (the notch's black capsule, or the 34-point one under the menu bar) it
/// hangs from.
pub(crate) const TAB_TOP: f32 = 66.0;
/// The tab's widest.
const TAB_MAX_W: f32 = 560.0;
const TAB_RADIUS: f32 = 18.0;

/// A frame overlay's state: one per display.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Frame {
    /// The display it covers (`NSScreenNumber`), while it shows.
    pub(crate) display: Option<u32>,
    /// The display's size in points.
    pub(crate) size: (f32, f32),
    /// Where it meets other displays.
    pub(crate) seams: Seams,
    /// The glow shows: the agent has the controls. Without it the panel
    /// shows only its tab ("Back to you").
    pub(crate) glow: bool,
    /// The tab, on the display it's about.
    pub(crate) tab: Option<Tab>,
    /// Seconds left of a borrow, for the tab's countdown.
    pub(crate) left: Option<u64>,
}

/// What the desk frame's tab says.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tab {
    /// A task's window brought in front for a moment.
    InFront {
        app: String,
        /// The window, so the tab shows on its display.
        window: TaskWindow,
        /// The guard's reason (`guard::BORROW_REASONS`).
        why: String,
        /// When it goes back at the latest (Unix ms).
        until_ms: u64,
    },
    /// A task moves the real pointer or presses real keys (in `app`).
    Hands {
        app: Option<String>,
        window: Option<TaskWindow>,
    },
    /// The front went back to the person's `app`; shown until `until`.
    Back { app: String, until: Instant },
}

impl Tab {
    /// The tab for what `view` says at `now`: a borrow first, then real
    /// input, then the moment after a borrow.
    pub(crate) fn of(view: &Stream, now: Instant) -> Option<Tab> {
        if let Some(task) = view.borrowing()
            && let Some(borrow) = &task.borrowed
        {
            return Some(Tab::InFront {
                app: borrow.window.app.clone(),
                window: borrow.window.clone(),
                why: borrow.why.clone(),
                until_ms: borrow.until_ms,
            });
        }
        if let Some(task) = view.hands_off() {
            let window = task.windows.first().cloned();
            return Some(Tab::Hands {
                app: window.as_ref().map(|window| window.app.clone()),
                window,
            });
        }
        let back = view
            .tasks
            .iter()
            .filter_map(|task| task.returned.as_ref())
            .filter(|(_, at)| now < *at + BACK_FOR)
            .max_by_key(|(_, at)| *at)?;
        Some(Tab::Back {
            app: back.0.clone(),
            until: back.1 + BACK_FOR,
        })
    }

    /// The window it's about, if any: the tab shows on its display.
    pub(crate) fn window(&self) -> Option<&TaskWindow> {
        match self {
            Tab::InFront { window, .. } => Some(window),
            Tab::Hands { window, .. } => window.as_ref(),
            Tab::Back { .. } => None,
        }
    }

    /// Whether the agent has the controls (the glow shows with it).
    pub(crate) fn in_front(&self) -> bool {
        !matches!(self, Tab::Back { .. })
    }

    /// Whether it still shows at `now`.
    pub(crate) fn shows(&self, now: Instant) -> bool {
        match self {
            Tab::Back { until, .. } => now < *until,
            _ => true,
        }
    }

    /// Seconds left of a borrow at `now_ms`, rounded up.
    pub(crate) fn left(&self, now_ms: u64) -> Option<u64> {
        match self {
            Tab::InFront { until_ms, .. } => Some(until_ms.saturating_sub(now_ms).div_ceil(1000)),
            _ => None,
        }
    }

    /// Its first line: who has what.
    pub(crate) fn line(&self) -> String {
        match self {
            Tab::InFront { app, .. } => tr_fill("Agent-Desk is using {} for a moment", app),
            Tab::Hands { app: Some(app), .. } => {
                tr_fill("Agent-Desk is using your mouse and keys in {}", app)
            }
            Tab::Hands { app: None, .. } => {
                tr("Agent-Desk is using your mouse and keys").to_owned()
            }
            Tab::Back { app, .. } => tr_fill("Back to you · {}", app),
        }
    }

    /// Its second line, while the agent has the controls: why (a borrow),
    /// how to stop it, and when it gives the front back.
    pub(crate) fn detail(&self, left: Option<u64>) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        match self {
            Tab::InFront { app, why, .. } => parts.push(why_words(why, app)),
            Tab::Hands { .. } => {}
            Tab::Back { .. } => return None,
        }
        parts.push(tr("Esc to stop").to_owned());
        if let Some(left) = left.filter(|left| *left > 0) {
            parts.push(tr_fill("back in {} s", &left.to_string()));
        }
        Some(parts.join(" · "))
    }
}

/// Why `app` came forward, in the person's language.
pub(crate) fn why_words(why: &str, app: &str) -> String {
    if why == crate::assistant::guard::BORROW_IGNORED {
        tr_fill("{} didn't respond in the background", app)
    } else {
        tr_fill("{} needs keys typed in front", app)
    }
}

pub(crate) fn view(frame: &Frame, cx: &mut Cx<Frame>) -> El {
    drawn(Paint::new(cx.system_dark(), cx.env().scale), frame)
}

/// The frame on its display: the glow while the agent has the controls,
/// and the tab hanging under the island.
pub(crate) fn drawn(p: Paint, frame: &Frame) -> El {
    let glow = frame
        .glow
        .then(|| glow_joined(p.k.dark, frame.size, &frame.seams));
    let tab = frame.tab.as_ref().map(|tab| {
        row()
            .key("frame-tab-row")
            .absolute(0.0, TAB_TOP)
            .w(frame.size.0)
            .justify_center()
            .child(tab_view(p, tab, frame.left))
    });
    el().key("frame-root")
        .w_full()
        .h_full()
        .children(glow)
        .children(tab)
}

/// The tab: a black capsule (the island's black, in both appearances)
/// with the agent's hand, who has what, and how to stop it.
pub(crate) fn tab_view(p: Paint, tab: &Tab, left: Option<u64>) -> El {
    let line = tab.line();
    let detail = tab.detail(left);
    // On black: the agent's lighter hue, white words.
    let hue = palette::agent(true);
    let glyph = if tab.in_front() {
        &look::HAND
    } else {
        &look::CHECK
    };
    let spoken = match &detail {
        Some(detail) => format!("{line}. {detail}. {}", tr("Touch anything to take over")),
        None => line.clone(),
    };
    let words = col()
        .gap(1.0)
        .min_w(0.0)
        .child(
            text(line)
                .key("frame-tab-line")
                .truncate()
                .font(look::island_line(p))
                .color(Color::WHITE)
                .a11y_hidden(),
        )
        .children(detail.map(|detail| {
            text(detail)
                .key("frame-tab-detail")
                .truncate()
                .font(look::meta(p))
                .color(Color::WHITE.alpha(0.66))
                .a11y_hidden()
        }));
    row()
        .key(if tab.in_front() {
            "frame-tab"
        } else {
            "frame-back"
        })
        .max_w(TAB_MAX_W)
        .px(16.0)
        .py(9.0)
        .gap(10.0)
        .center_y()
        .radius(TAB_RADIUS)
        .fill(Color::BLACK)
        .border(p.hair, Color::WHITE.alpha(0.14))
        .box_shadow(look::shadow(p))
        .role("status")
        .label(spoken)
        .live_region(if tab.in_front() {
            rune::LiveRegion::Assertive
        } else {
            rune::LiveRegion::Polite
        })
        .child(icon(glyph, 16.0).color(hue).a11y_hidden())
        .child(words)
}

/// The frame on a display alone, filling its parent.
pub(crate) fn glow(dark: bool) -> El {
    ring(dark)
}

/// The frame on a display of `size` meeting others along `seams`.
pub(crate) fn glow_joined(dark: bool, size: (f32, f32), seams: &Seams) -> El {
    if !seams.joined.iter().any(|joined| *joined) {
        return glow(dark);
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
    el().key("frame-joined")
        .w_full()
        .h_full()
        .clip_rounded(0.0)
        .a11y_hidden()
        .child(ring)
        .children(strips)
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
