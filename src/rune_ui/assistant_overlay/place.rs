//! Where the rail sits (`docs/ux/redesign.md` §3.1): top right of the
//! pointer's display, 18 points from its edge, just under the menu bar,
//! where macOS puts notifications; wherever the person dragged it on that
//! display once they have, remembered per display across launches
//! (`<support>/assistant/orb.json`). It never covers an outlined window:
//! when one is under it, it flips to the display's left edge
//! ([`clear_of`], [`anchor_clear`]). Which way its cards and the list
//! unfold follows from where it sits, so they always grow into the
//! display. Plain geometry here; the host reads the screens and the
//! watcher's outline rects.

use std::collections::HashMap;
use std::sync::Mutex;

use super::look;
use super::orb::{self, Grow};

/// A display's visible area (below the menu bar, beside the Dock) in
/// global points, y down: x, y, width, height.
pub(crate) type Area = [f64; 4];

/// From the display's edge to the rail's cards.
const RAIL_EDGE: f64 = 18.0;
/// From the menu bar to the rail's head.
const RAIL_TOP: f64 = 12.0;
/// How far the anchor may come to the display's edge (clamping a drag).
const EDGE: f64 = 24.0;
/// The orb's core centre from the panel's corner on its side.
const CORE_IN: (f64, f64) = (
    orb::INSET as f64 + 15.0,
    orb::INSET as f64 + orb::ORB_H as f64 / 2.0,
);

/// The rail's default anchor on a display: top right, its cards
/// [`RAIL_EDGE`] from the right edge and its head [`RAIL_TOP`] under the
/// menu bar. `hud` is no longer read (the rail no longer sits beside it);
/// it stays so the callers keep their shape.
pub(crate) fn default_anchor(visible: Area, hud: Option<(f64, bool)>) -> (f64, f64) {
    let _ = hud;
    let [x, y, w, _] = visible;
    // The head's outer end sits CORE_IN.0 - INSET past the anchor.
    let inward = CORE_IN.0 - f64::from(orb::INSET);
    let down = CORE_IN.1 - f64::from(orb::INSET);
    clamp((x + w - RAIL_EDGE - inward, y + RAIL_TOP + down), visible)
}

/// About how tall the rail stands for `board` (the list unfolded: all of
/// `room`), with at most `cards` floating cards (`orb::rail_cards`): its
/// head, its question, the tasks the person holds, the result, the job
/// rows, and the "N more" line.
pub(crate) fn rail_height(
    board: &super::board::Board,
    expanded: bool,
    room: f64,
    cards: usize,
) -> f64 {
    use super::board::Phase;
    const HEAD: f64 = 52.0;
    const QUESTION: f64 = 178.0;
    const STANDING: f64 = 44.0;
    const RESULT: f64 = 136.0;
    const ROW: f64 = 104.0;
    const MORE: f64 = 34.0;
    if expanded {
        return room;
    }
    let mut left = cards;
    let mut take = |n: usize| {
        let n = n.min(left);
        left -= n;
        n as f64
    };
    let question = take(usize::from(board.active_question().is_some()));
    let standing = take(
        board
            .stack()
            .iter()
            .filter(|task| matches!(task.phase, Phase::Paused | Phase::Waiting { .. }))
            .count(),
    );
    let results = take(board.pops.items.len().min(orb::RAIL_RESULTS));
    let running = super::jobs::running(&board.tasks);
    let layout = super::jobs::layout_at_most(&running, question > 0.0, left);
    let more = if layout.more > 0 { MORE } else { 0.0 };
    (HEAD
        + question * QUESTION
        + standing * STANDING
        + results * RESULT
        + layout.rows.len() as f64 * ROW
        + more)
        .min(room)
}

/// What the rail's cards cover for an anchor unfolding `grow`, `height`
/// tall below its head's top: x, y, width, height (global points).
pub(crate) fn footprint((ax, ay): (f64, f64), grow: Grow, height: f64) -> Area {
    let inward = CORE_IN.0 - f64::from(orb::INSET);
    let width = f64::from(look::RAIL_W);
    let x = if grow.right {
        ax - inward
    } else {
        ax + inward - width
    };
    let top = ay - (CORE_IN.1 - f64::from(orb::INSET));
    let y = if grow.down {
        top
    } else {
        ay + (CORE_IN.1 - f64::from(orb::INSET)) - height
    };
    [x, y, width, height]
}

pub(crate) fn overlaps([ax, ay, aw, ah]: Area, [bx, by, bw, bh]: Area) -> bool {
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

/// How much of `a` lies over `b`, in square points.
fn overlap_area([ax, ay, aw, ah]: Area, [bx, by, bw, bh]: Area) -> f64 {
    let w = (ax + aw).min(bx + bw) - ax.max(bx);
    let h = (ay + ah).min(by + bh) - ay.max(by);
    w.max(0.0) * h.max(0.0)
}

/// Something a card keeps clear of (`docs/ux/redesign.md` §3.1): the
/// focus window's title bar and the other cards are `Hard`; an outlined
/// window's label is `Label`, whole if it can be, else at least its dot
/// and the start of its title (the label then shortens beside the card,
/// [`label_room`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Keep {
    Hard(Area),
    Label { whole: Area, least: Area },
}

/// A panel at `want` (its card `inset` in from each side) moved sideways
/// just clear of the rail (`rail`) when its card would run under it,
/// kept on `within` (the stage stays under the island where it can).
pub(crate) fn beside_rail(want: Area, inset: f64, rail: Area, within: Area) -> Area {
    const GAP: f64 = 12.0;
    let [x, y, w, h] = want;
    let card = [x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset];
    if !overlaps(card, rail) {
        return want;
    }
    let rail_mid = rail[0] + rail[2] / 2.0;
    let x = if rail_mid > card[0] + card[2] / 2.0 {
        // The rail is to the right: end just before it.
        rail[0] - GAP - card[2] - inset
    } else {
        rail[0] + rail[2] + GAP - inset
    };
    [
        x.clamp(
            within[0] - inset,
            (within[0] + within[2] - w + inset).max(within[0] - inset),
        ),
        y,
        w,
        h,
    ]
}

/// How far a card may move down from where it wants to be before a label
/// may shorten for it instead.
const NEAR: f64 = 96.0;
/// The steps a card moves in, looking for room.
const STEP: f64 = 8.0;

/// Where a card `size` big goes (its top-left): `want` if that covers
/// nothing in `keep`, else the nearest spot within `within` that doesn't,
/// sliding sideways first, then down (a card hangs under the island; it
/// never rises over it). A label stays whole when a spot within [`NEAR`]
/// allows it; past that, a label may shorten to its least. With no clear
/// spot at all, the one covering least.
pub(crate) fn clear_spot(
    want: (f64, f64),
    size: (f64, f64),
    within: Area,
    keep: &[Keep],
) -> (f64, f64) {
    let [wx, wy, ww, wh] = within;
    let (w, h) = size;
    let x_lo = wx;
    let x_hi = (wx + ww - w).max(wx);
    let y_hi = (wy + wh - h).max(want.1);
    let at = |dx: f64, dy: f64| ((want.0 + dx).clamp(x_lo, x_hi), want.1 + dy);
    let card = |(x, y): (f64, f64)| [x, y, w, h];
    let covers = |spot: (f64, f64), whole: bool| -> f64 {
        keep.iter()
            .map(|keep| match keep {
                Keep::Hard(area) => overlap_area(card(spot), *area),
                Keep::Label { whole: all, least } => {
                    overlap_area(card(spot), if whole { *all } else { *least })
                }
            })
            .sum()
    };
    // Candidates nearest first: a step down costs as much as two sideways.
    let mut spots: Vec<(f64, (f64, f64))> = Vec::new();
    let mut dy = 0.0;
    while want.1 + dy <= y_hi + 0.01 {
        let mut dx = 0.0;
        while dx <= ww {
            for sign in [1.0, -1.0] {
                let spot = at(sign * dx, dy);
                spots.push((dx + 2.0 * dy, spot));
                if dx == 0.0 {
                    break;
                }
            }
            dx += STEP;
        }
        dy += STEP;
    }
    spots.sort_by(|a, b| a.0.total_cmp(&b.0));
    let near = |spot: &(f64, f64)| spot.1 - want.1 <= NEAR;
    if let Some((_, spot)) = spots
        .iter()
        .find(|(_, spot)| near(spot) && covers(*spot, true) == 0.0)
    {
        return *spot;
    }
    if let Some((_, spot)) = spots.iter().find(|(_, spot)| covers(*spot, false) == 0.0) {
        return *spot;
    }
    spots
        .iter()
        .min_by(|a, b| covers(a.1, false).total_cmp(&covers(b.1, false)))
        .map_or(want, |(_, spot)| *spot)
}

/// How wide a label starting at `label` (its whole rect) may draw so that
/// none of `cards` covers it: up to the first card that sits over its
/// line to its right; `None` when no card is in the way (or one covers
/// its start: shortening wouldn't show it).
pub(crate) fn label_room(label: Area, cards: &[Area]) -> Option<f64> {
    let [lx, _, _, _] = label;
    cards
        .iter()
        .filter(|card| overlaps(label, **card) && card[0] > lx + 24.0)
        .map(|card| card[0] - lx - 6.0)
        .fold(None, |room: Option<f64>, each| {
            Some(room.map_or(each, |room| room.min(each)))
        })
        .map(|room| room.max(0.0))
}

/// `anchor`, flipped to the display's other side when the rail there
/// (`height` tall) would cover one of the `outlines` (the windows a live
/// task works in, as the watcher outlines them) and the other side is
/// clear of them; else `anchor` as it is.
pub(crate) fn clear_of(
    anchor: (f64, f64),
    visible: Area,
    outlines: &[Area],
    height: f64,
) -> (f64, f64) {
    let covers = |at: (f64, f64)| {
        let rail = footprint(at, grow(at, visible), height);
        outlines.iter().any(|outline| overlaps(rail, *outline))
    };
    if !covers(anchor) {
        return anchor;
    }
    let [x, _, w, _] = visible;
    let mirrored = clamp((x + (x + w - anchor.0), anchor.1), visible);
    if !covers(mirrored) {
        return mirrored;
    }
    // Outlined windows on both sides: the side covering fewer of their
    // labels (the strip above each window's top), else where it is. The
    // labels it still lies over shorten beside it (`label_room`).
    let labels = |at: (f64, f64)| {
        let rail = footprint(at, grow(at, visible), height);
        outlines
            .iter()
            .filter(|[ox, oy, ow, _]| overlaps(rail, [*ox, oy - 30.0, ow.min(240.0), 30.0]))
            .count()
    };
    if labels(mirrored) < labels(anchor) {
        mirrored
    } else {
        anchor
    }
}

/// The rail's anchor on the display keyed `key`, with the flip rule: where
/// the person dragged it there stays theirs; the default top right flips
/// left when an outlined window is under it. The host passes the watcher's
/// outline rects and about how tall the rail stands (`host::follow`).
pub(crate) fn anchor_clear(
    key: &str,
    visible: Area,
    hud: Option<(f64, bool)>,
    outlines: &[Area],
    height: f64,
) -> (f64, f64) {
    let dragged = with_dragged(|dragged| dragged.get(key).copied());
    match dragged {
        Some(_) => placed(dragged, visible, hud),
        None => clear_of(default_anchor(visible, hud), visible, outlines, height),
    }
}

/// `anchor` kept far enough inside `visible` that the orb shows whole.
pub(crate) fn clamp((ax, ay): (f64, f64), visible: Area) -> (f64, f64) {
    let [x, y, w, h] = visible;
    let room = EDGE.min(w / 2.0).min(h / 2.0);
    (
        ax.clamp(x + room, x + w - room),
        ay.clamp(y + room, y + h - room),
    )
}

/// Which way things unfold from an orb at `anchor`: down from the top half,
/// up from the bottom half; right unless it's in the right quarter.
pub(crate) fn grow((ax, ay): (f64, f64), visible: Area) -> Grow {
    let [x, y, w, h] = visible;
    Grow {
        down: ay < y + h / 2.0,
        right: ax < x + w * 0.75,
    }
}

/// The orb panel's rect (global points) for an orb centred at `anchor`
/// unfolding `grow`.
pub(crate) fn rect((ax, ay): (f64, f64), grow: Grow) -> [f32; 4] {
    let (w, h) = (f64::from(orb::SIZE.0), f64::from(orb::SIZE.1));
    let x = if grow.right {
        ax - CORE_IN.0
    } else {
        ax + CORE_IN.0 - w
    };
    let y = if grow.down {
        ay - CORE_IN.1
    } else {
        ay + CORE_IN.1 - h
    };
    [x as f32, y as f32, orb::SIZE.0, orb::SIZE.1]
}

/// [`rect`], no taller than the room `visible` has past the orb the way
/// things unfold, so nothing hangs off the display: the panel says its
/// height to the view ([`orb::Orb::room`]), where the rows scroll in it.
pub(crate) fn fitted(anchor: (f64, f64), grow: Grow, visible: Area) -> [f32; 4] {
    let [x, y, w, h] = rect(anchor, grow);
    let [_, top, _, height] = visible;
    let room = if grow.down {
        top + height - f64::from(y)
    } else {
        f64::from(y + h) - top
    };
    let fit = (room as f32).clamp(orb::ORB_H + 2.0 * orb::INSET, h);
    let y = if grow.down { y } else { y + h - fit };
    [x, y, w, fit]
}

/// Where the person dragged the orb on each display: its centre from the
/// visible area's top-left, by the display's lasting key (its UUID).
static DRAGGED: Mutex<Option<Dragged>> = Mutex::new(None);

type Dragged = HashMap<String, (f64, f64)>;

fn with_dragged<T>(f: impl FnOnce(&mut Dragged) -> T) -> T {
    let mut dragged = DRAGGED.lock().unwrap_or_else(|error| error.into_inner());
    f(dragged.get_or_insert_with(load))
}

/// The orb's centre from where it was dragged on a display (`dragged`, from
/// its visible area's top-left), else top right, kept on the display.
pub(crate) fn placed(
    dragged: Option<(f64, f64)>,
    visible: Area,
    hud: Option<(f64, bool)>,
) -> (f64, f64) {
    match dragged {
        Some((dx, dy)) => clamp((visible[0] + dx, visible[1] + dy), visible),
        None => default_anchor(visible, hud),
    }
}

/// Remembers that the orb was dropped at `anchor` on the display keyed
/// `key`.
pub(crate) fn remember(key: &str, visible: Area, anchor: (f64, f64)) {
    let (ax, ay) = clamp(anchor, visible);
    let offset = (ax - visible[0], ay - visible[1]);
    let saved = with_dragged(|dragged| {
        dragged.insert(key.to_owned(), offset);
        dragged.clone()
    });
    // Off the main thread: a write never holds up the drop.
    let _ = std::thread::Builder::new()
        .name("hex-orb-place".into())
        .spawn(move || save(&saved));
}

fn file() -> Option<std::path::PathBuf> {
    Some(
        crate::assistant::store::assistant_dir()
            .ok()?
            .join("orb.json"),
    )
}

fn load() -> Dragged {
    file()
        .and_then(|path| std::fs::read(path).ok())
        .map(|bytes| parse(&bytes))
        .unwrap_or_default()
}

fn parse(bytes: &[u8]) -> Dragged {
    serde_json::from_slice::<HashMap<String, [f64; 2]>>(bytes)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, [x, y])| x.is_finite() && y.is_finite())
        .map(|(key, [x, y])| (key, (x, y)))
        .collect()
}

fn save(dragged: &Dragged) {
    let Some(path) = file() else {
        return;
    };
    let json: HashMap<&str, [f64; 2]> = dragged
        .iter()
        .map(|(key, (x, y))| (key.as_str(), [*x, *y]))
        .collect();
    if let Ok(bytes) = serde_json::to_vec(&json)
        && let Err(error) = std::fs::write(&path, bytes)
    {
        tracing::warn!(%error, "assistant: couldn't remember where the orb was dragged");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1512 × 982 display with a 33-point menu bar and a 70-point Dock.
    const VISIBLE: Area = [0.0, 33.0, 1512.0, 879.0];

    #[test]
    fn the_rail_sits_top_right_under_the_menu_bar() {
        let anchor = default_anchor(VISIBLE, Some((756.0, true)));
        let toward = grow(anchor, VISIBLE);
        assert_eq!(
            toward,
            Grow {
                down: true,
                right: false
            },
            "it unfolds down and to the left, into the display"
        );
        let [x, y, w, _] = footprint(anchor, toward, 400.0);
        assert!((1512.0 - (x + w) - RAIL_EDGE).abs() < 0.01, "{x} {w}");
        assert!((y - (33.0 + RAIL_TOP)).abs() < 0.01, "{y}");
        // Wherever the HUD is.
        assert_eq!(default_anchor(VISIBLE, Some((756.0, false))), anchor);
        assert_eq!(default_anchor(VISIBLE, None), anchor);
    }

    #[test]
    fn rail_never_covers_an_outlined_window() {
        let height = 600.0;
        let anchor = default_anchor(VISIBLE, None);
        let rail = footprint(anchor, grow(anchor, VISIBLE), height);
        // Nothing outlined, or an outline elsewhere: it stays top right.
        assert_eq!(clear_of(anchor, VISIBLE, &[], height), anchor);
        let left_window = [40.0, 80.0, 700.0, 500.0];
        assert_eq!(clear_of(anchor, VISIBLE, &[left_window], height), anchor);
        // An outlined window under it: it flips to the left edge, clear of it.
        for window in [
            [900.0, 60.0, 600.0, 700.0],
            [1300.0, 600.0, 200.0, 200.0],
            [rail[0] + 10.0, rail[1] + 10.0, 20.0, 20.0],
        ] {
            let moved = clear_of(anchor, VISIBLE, &[window], height);
            let toward = grow(moved, VISIBLE);
            assert!(
                toward.right,
                "{window:?}: unfolds rightward from the left edge"
            );
            let there = footprint(moved, toward, height);
            assert!(!overlaps(there, window), "{window:?}: {there:?}");
            assert!((there[0] - RAIL_EDGE).abs() < 0.01, "{there:?}");
            assert_eq!(moved.1, anchor.1, "still under the menu bar");
        }
        // Outlined windows on both sides: it keeps its place rather than
        // jump between them.
        let both = [[900.0, 60.0, 600.0, 700.0], [0.0, 60.0, 600.0, 700.0]];
        assert_eq!(clear_of(anchor, VISIBLE, &both, height), anchor);
    }

    #[test]
    fn a_card_slides_off_a_label_and_a_title_bar_and_labels_shorten_last() {
        let within = [0.0, 24.0, 1280.0, 776.0];
        let size = (482.0, 200.0);
        let want = (399.0, 60.0);
        // Nothing in the way: where it wants to be.
        assert_eq!(clear_spot(want, size, within, &[]), want);
        // A label under it: it slides aside (or down) until clear.
        let label = [300.0, 230.0, 380.0, 22.0];
        let keep = [Keep::Label {
            whole: label,
            least: [300.0, 230.0, 120.0, 22.0],
        }];
        let (x, y) = clear_spot(want, size, within, &keep);
        assert!(!overlaps([x, y, size.0, size.1], label), "{x} {y}");
        // The focus window's title bar right under the island, and labels
        // either side: it goes down past the bar, never up over the island.
        let bar = [0.0, 100.0, 1280.0, 30.0];
        let (x, y) = clear_spot(want, size, within, &[Keep::Hard(bar)]);
        assert!(y >= 130.0, "{x} {y}");
        // No clear spot near: a label may shorten to its least.
        let wide = [0.0, 60.0, 1280.0, 22.0];
        let keep = [
            Keep::Label {
                whole: wide,
                least: [0.0, 60.0, 120.0, 22.0],
            },
            Keep::Hard([0.0, 82.0, 1280.0, 700.0]),
        ];
        let (x, y) = clear_spot((399.0, 60.0), (482.0, 20.0), within, &keep);
        assert!(x >= 120.0 && y < 82.0, "{x} {y}");
        // A label shortens beside a card to its right, not one over its start.
        assert_eq!(
            label_room(label, &[[600.0, 0.0, 300.0, 400.0]]),
            Some(294.0)
        );
        assert_eq!(label_room(label, &[[200.0, 0.0, 300.0, 400.0]]), None);
        assert_eq!(label_room(label, &[[900.0, 0.0, 300.0, 400.0]]), None);
    }

    #[test]
    fn the_stage_moves_aside_off_the_rail() {
        let within = [0.0, 24.0, 1280.0, 776.0];
        let rail = [914.0, 36.0, 348.0, 500.0];
        let want = [296.0, 44.0, 688.0, 660.0];
        let [x, ..] = beside_rail(want, 24.0, rail, within);
        assert!(x + 688.0 - 24.0 <= 914.0 - 12.0 + 0.01, "{x}");
        // Clear of it already: it stays.
        assert_eq!(
            beside_rail(want, 24.0, [1000.0, 36.0, 200.0, 100.0], within),
            want
        );
    }

    #[test]
    fn the_panel_puts_the_orb_on_its_anchor_and_unfolds_into_the_display() {
        for grow in [
            Grow {
                down: true,
                right: true,
            },
            Grow {
                down: false,
                right: false,
            },
        ] {
            let [x, y, w, h] = rect((1000.0, 500.0), grow);
            let core_x = if grow.right {
                x as f64 + CORE_IN.0
            } else {
                (x + w) as f64 - CORE_IN.0
            };
            let core_y = if grow.down {
                y as f64 + CORE_IN.1
            } else {
                (y + h) as f64 - CORE_IN.1
            };
            assert_eq!((core_x, core_y), (1000.0, 500.0));
        }
        let corner = clamp((5000.0, -40.0), VISIBLE);
        assert_eq!(corner, (1512.0 - EDGE, 33.0 + EDGE));
        assert_eq!(
            grow(corner, VISIBLE),
            Grow {
                down: true,
                right: false
            }
        );
    }

    #[test]
    fn remembered_spots_read_back_and_bad_ones_are_dropped() {
        let parsed = parse(br#"{"1": [900.5, 40], "x": [1, 2], "2": [1, 2, 3]}"#);
        assert_eq!(parsed.len(), 0, "a malformed file is ignored whole");
        let parsed = parse(br#"{"37D8832A-2D66": [900.5, 40], "7": [10, 20]}"#);
        assert_eq!(parsed.get("37D8832A-2D66"), Some(&(900.5, 40.0)));
        assert_eq!(parsed.get("7"), Some(&(10.0, 20.0)));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn on_this_macs_two_displays_the_orb_sits_beside_the_hud_on_each() {
        let screens = super::super::screens::tests::two();
        let (built_in, external) = (screens[0].visible, screens[1].visible);
        let inside = |[x, y, w, h]: [f32; 4], [fx, fy, fw, fh]: [f64; 4]| {
            f64::from(x) >= fx
                && f64::from(y) >= fy
                && f64::from(x + w) <= fx + fw
                && f64::from(y + h) <= fy + fh
        };
        // Top right of the built-in display.
        let here = default_anchor(built_in, Some((900.0, true)));
        let toward = grow(here, built_in);
        assert!(toward.down && !toward.right, "{toward:?}");
        assert!(inside(rect(here, toward), screens[0].frame));
        let [x, _, w, _] = footprint(here, toward, 100.0);
        assert!((built_in[0] + built_in[2] - (x + w) - RAIL_EDGE).abs() < 0.01);
        // Top right of the M27Q above.
        let there = default_anchor(external, Some((676.0, true)));
        assert!(there.1 < here.1, "on the display above");
        assert!(inside(rect(there, grow(there, external)), screens[1].frame));
        // Dragged to the M27Q's lower right: kept there, unfolding up and
        // left into it.
        let dragged = placed(Some((2400.0, 1300.0)), external, None);
        assert_eq!(dragged, (1796.0, -115.0));
        let toward = grow(dragged, external);
        assert_eq!(
            toward,
            Grow {
                down: false,
                right: false
            }
        );
        assert!(inside(rect(dragged, toward), screens[1].frame));
        // A spot remembered on a bigger display, shown on the built-in one:
        // kept on it.
        let squeezed = placed(Some((2400.0, 1300.0)), built_in, None);
        assert_eq!(squeezed, (1800.0 - EDGE, 37.0 + 1132.0 - EDGE));
    }

    #[test]
    fn the_panel_never_hangs_off_its_display() {
        // Beside the HUD at the top: the tallest panel fits.
        let top = default_anchor(VISIBLE, Some((756.0, true)));
        let [_, y, _, h] = fitted(top, grow(top, VISIBLE), VISIBLE);
        assert_eq!(h, orb::SIZE.1.min(879.0 + 33.0 - y));
        assert!(f64::from(y + h) <= 33.0 + 879.0);
        // Just above the middle, unfolding down: only the room left.
        for anchor in [(400.0, 460.0), (400.0, 480.0), (1400.0, 880.0)] {
            let toward = grow(anchor, VISIBLE);
            let [_, y, _, h] = fitted(anchor, toward, VISIBLE);
            assert!(f64::from(y) >= 33.0 - 0.01, "{anchor:?}: {y}");
            assert!(
                f64::from(y + h) <= 33.0 + 879.0 + 0.01,
                "{anchor:?}: {y} {h}"
            );
            assert!(h > 400.0, "still roomy: {h}");
            // The orb stays on its anchor.
            let core = if toward.down {
                f64::from(y) + CORE_IN.1
            } else {
                f64::from(y + h) - CORE_IN.1
            };
            assert!((core - anchor.1).abs() < 0.01, "{core} {anchor:?}");
        }
    }
}
