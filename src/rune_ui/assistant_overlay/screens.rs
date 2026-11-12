//! Displays as the overlays see them, and the geometry that keeps every
//! overlay right on a desk of several: which display the person works on,
//! where a display's frame meets another's (so the glow runs on across the
//! seam instead of doubling there), how a window's outline splits into one
//! piece per display it spans (each panel then lies on one display, drawn
//! at that display's scale), and how a spot past the HUD carries over to
//! another display. Plain geometry in global points (y down from the
//! primary display's top-left, Rune's and Accessibility's space), so every
//! rule is a unit test; the host reads the real displays.

/// A rect in global points: x, y, width, height.
pub(crate) type Rect = [f64; 4];

/// Two edges closer than this touch.
const TOUCH: f64 = 1.0;

/// One display.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Screen {
    /// `CGDirectDisplayID` (`NSScreenNumber`), what Rune pins panels by;
    /// it can change when a display is plugged into another port.
    pub(crate) id: u32,
    /// A key that stays with the display across ports and restarts (its
    /// UUID), for what's remembered per display.
    pub(crate) key: String,
    pub(crate) frame: Rect,
    /// Below the menu bar and beside the Dock.
    pub(crate) visible: Rect,
    /// Pixels per point.
    pub(crate) scale: f64,
}

impl Screen {
    /// Whether `point` is on it: closed at its top and left, open at its
    /// bottom and right, so a point on a shared edge is on one display.
    pub(crate) fn contains(&self, (x, y): (f64, f64)) -> bool {
        let [sx, sy, sw, sh] = self.frame;
        x >= sx && x < sx + sw && y >= sy && y < sy + sh
    }
}

/// The overlap of two rects, if they overlap at all.
pub(crate) fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    (right > x && bottom > y).then_some([x, y, right - x, bottom - y])
}

fn area(rect: Option<Rect>) -> f64 {
    rect.map_or(0.0, |[_, _, w, h]| w * h)
}

/// The display most of `rect` is on, else the one nearest its centre.
pub(crate) fn under(screens: &[Screen], rect: Rect) -> Option<&Screen> {
    let centre = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    screens
        .iter()
        .max_by(|a, b| area(intersect(a.frame, rect)).total_cmp(&area(intersect(b.frame, rect))))
        .filter(|screen| area(intersect(screen.frame, rect)) > 0.0)
        .or_else(|| nearest(screens, centre))
}

/// The display `point` is on, else the nearest one.
pub(crate) fn at(screens: &[Screen], point: (f64, f64)) -> Option<&Screen> {
    screens
        .iter()
        .find(|screen| screen.contains(point))
        .or_else(|| nearest(screens, point))
}

fn nearest(screens: &[Screen], (x, y): (f64, f64)) -> Option<&Screen> {
    let distance = |screen: &&Screen| {
        let [sx, sy, sw, sh] = screen.frame;
        let dx = (sx - x).max(x - (sx + sw)).max(0.0);
        let dy = (sy - y).max(y - (sy + sh)).max(0.0);
        dx * dx + dy * dy
    };
    screens
        .iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
}

/// The display the person works on: the one under the window in front,
/// else the pointer's.
pub(crate) fn working(
    screens: &[Screen],
    front_window: Option<Rect>,
    pointer: (f64, f64),
) -> Option<&Screen> {
    front_window
        .and_then(|window| under(screens, window))
        .or_else(|| at(screens, pointer))
}

/// A display's edges, in the order a frame's lists them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

impl Edge {
    pub(crate) const ALL: [Edge; 4] = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];

    fn index(self) -> usize {
        self as usize
    }
}

/// A stretch of one edge, in points along it from its start (left for top
/// and bottom, top for left and right).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Strip {
    pub(crate) edge: Edge,
    pub(crate) from: f32,
    pub(crate) to: f32,
}

/// Where a display's frame meets its neighbours'.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Seams {
    /// Each edge (`Edge::ALL` order) that touches another display at all:
    /// the frame's glow moves off it, so two displays don't both glow at
    /// the seam.
    pub(crate) joined: [bool; 4],
    /// The open stretches of joined edges, which glow on their own.
    pub(crate) open: Vec<Strip>,
}

impl Seams {
    pub(crate) fn joined(&self, edge: Edge) -> bool {
        self.joined[edge.index()]
    }
}

/// Where `screen` meets the other `screens`.
pub(crate) fn seams(screen: &Screen, screens: &[Screen]) -> Seams {
    let [x, y, w, h] = screen.frame;
    let mut seams = Seams::default();
    for edge in Edge::ALL {
        // The edge as a line: where it sits across, and its span along.
        let (at, start, len) = match edge {
            Edge::Top => (y, x, w),
            Edge::Bottom => (y + h, x, w),
            Edge::Left => (x, y, h),
            Edge::Right => (x + w, y, h),
        };
        let mut touching: Vec<(f64, f64)> = screens
            .iter()
            .filter(|other| other.id != screen.id)
            .filter_map(|other| {
                let [ox, oy, ow, oh] = other.frame;
                // The other's facing edge, and its span along ours.
                let (facing, from, to) = match edge {
                    Edge::Top => (oy + oh, ox, ox + ow),
                    Edge::Bottom => (oy, ox, ox + ow),
                    Edge::Left => (ox + ow, oy, oy + oh),
                    Edge::Right => (ox, oy, oy + oh),
                };
                let from = from.max(start);
                let to = to.min(start + len);
                ((facing - at).abs() < TOUCH && to - from > TOUCH).then_some((from, to))
            })
            .collect();
        if touching.is_empty() {
            continue;
        }
        seams.joined[edge.index()] = true;
        touching.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut cursor = start;
        for (from, to) in touching {
            if from - cursor > TOUCH {
                seams.open.push(Strip {
                    edge,
                    from: (cursor - start) as f32,
                    to: (from - start) as f32,
                });
            }
            cursor = cursor.max(to);
        }
        if start + len - cursor > TOUCH {
            seams.open.push(Strip {
                edge,
                from: (cursor - start) as f32,
                to: len as f32,
            });
        }
    }
    seams
}

/// `rect` split into one piece per display it lies on, each with that
/// display's id, snapped to its pixels; the whole of it on one display
/// (or none, then the nearest) is one piece.
pub(crate) fn pieces(rect: Rect, screens: &[Screen]) -> Vec<(u32, Rect)> {
    let on: Vec<(&Screen, Rect)> = screens
        .iter()
        .filter_map(|screen| intersect(screen.frame, rect).map(|part| (screen, part)))
        .collect();
    if on.len() <= 1 {
        return under(screens, rect)
            .map(|screen| (screen.id, snap(rect, screen.scale)))
            .into_iter()
            .collect();
    }
    on.into_iter()
        .map(|(screen, part)| (screen.id, snap(part, screen.scale)))
        .collect()
}

/// `rect` with its edges on whole device pixels at `scale`, so a panel
/// draws crisp on a 1× display as on a 2× one.
pub(crate) fn snap([x, y, w, h]: Rect, scale: f64) -> Rect {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let round = |v: f64| (v * scale).round() / scale;
    let (left, top) = (round(x), round(y));
    [left, top, round(x + w) - left, round(y + h) - top]
}

/// A spot past the HUD on `from` (global points; `below` when it hangs
/// under a HUD at the top), carried to the same place on `to`: the same
/// offset from the top centre (or the bottom centre for a HUD at the
/// bottom), kept inside `to`.
pub(crate) fn carry((x, y): (f64, f64), below: bool, from: &Screen, to: &Screen) -> (f64, f64) {
    let [fx, fy, fw, fh] = from.frame;
    let [tx, ty, tw, th] = to.frame;
    let dx = x - (fx + fw / 2.0);
    let ny = if below {
        ty + (y - fy)
    } else {
        ty + th - (fy + fh - y)
    };
    let nx = tx + tw / 2.0 + dx;
    (nx.clamp(tx, tx + tw), ny.clamp(ty, ty + th))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn screen(id: u32, frame: Rect, scale: f64) -> Screen {
        Screen {
            id,
            key: format!("uuid-{id}"),
            frame,
            visible: frame,
            scale,
        }
    }

    /// This Mac: the built-in display (primary, 1800 × 1169) and an M27Q
    /// (2560 × 1440) above it and to its left, both at 2×.
    pub(crate) fn two() -> Vec<Screen> {
        vec![
            Screen {
                visible: [0.0, 37.0, 1800.0, 1169.0 - 37.0],
                ..screen(1, [0.0, 0.0, 1800.0, 1169.0], 2.0)
            },
            Screen {
                visible: [-604.0, -1440.0 + 25.0, 2560.0, 1440.0 - 25.0 - 70.0],
                ..screen(2, [-604.0, -1440.0, 2560.0, 1440.0], 2.0)
            },
        ]
    }

    /// A 1920 × 1080 display at 1× to the built-in display's left, its top
    /// 100 points lower, beside the two above.
    fn three() -> Vec<Screen> {
        let mut screens = two();
        screens.push(screen(3, [-1920.0, 100.0, 1920.0, 1080.0], 1.0));
        screens
    }

    #[test]
    fn the_two_displays_meet_along_the_built_in_displays_whole_top() {
        let screens = two();
        let built_in = seams(&screens[0], &screens);
        assert!(built_in.joined(Edge::Top));
        assert!(built_in.open.is_empty(), "covered all along: {built_in:?}");
        assert!(!built_in.joined(Edge::Left) && !built_in.joined(Edge::Bottom));
        let external = seams(&screens[1], &screens);
        assert!(external.joined(Edge::Bottom));
        assert_eq!(
            external.open,
            [
                Strip {
                    edge: Edge::Bottom,
                    from: 0.0,
                    to: 604.0
                },
                Strip {
                    edge: Edge::Bottom,
                    from: 2404.0,
                    to: 2560.0
                },
            ],
            "the M27Q's bottom glows where it overhangs the built-in display"
        );
        assert!(!external.joined(Edge::Top) && !external.joined(Edge::Right));
    }

    #[test]
    fn a_third_display_beside_them_joins_where_it_touches() {
        let screens = three();
        let left = seams(&screens[2], &screens);
        assert!(left.joined(Edge::Right));
        assert_eq!(
            left.open,
            [Strip {
                edge: Edge::Right,
                from: 1069.0,
                to: 1080.0
            }],
            "its right edge runs 11 points past the built-in display's bottom"
        );
        let built_in = seams(&screens[0], &screens);
        assert!(built_in.joined(Edge::Left));
        assert_eq!(
            built_in.open,
            [Strip {
                edge: Edge::Left,
                from: 0.0,
                to: 100.0
            }]
        );
        // The M27Q's left overhang doesn't reach the third display.
        assert!(!seams(&screens[1], &screens).joined(Edge::Left));
    }

    #[test]
    fn the_working_display_is_the_front_windows_else_the_pointers() {
        let screens = two();
        let on_external = [100.0, -900.0, 1200.0, 700.0];
        assert_eq!(
            working(&screens, Some(on_external), (10.0, 10.0))
                .unwrap()
                .id,
            2
        );
        // Mostly on the built-in display, a little on the M27Q.
        let spanning = [200.0, -100.0, 900.0, 700.0];
        assert_eq!(
            working(&screens, Some(spanning), (0.0, -500.0)).unwrap().id,
            1
        );
        assert_eq!(working(&screens, None, (-300.0, -20.0)).unwrap().id, 2);
        // Off every display: the nearest.
        assert_eq!(at(&screens, (2500.0, 1100.0)).unwrap().id, 1);
        // On the shared edge: the display below it.
        assert_eq!(at(&screens, (900.0, 0.0)).unwrap().id, 1);
    }

    #[test]
    fn a_window_spanning_both_displays_outlines_in_one_piece_on_each() {
        let screens = two();
        let outline = [180.0, -224.0, 920.0, 644.0];
        let split = pieces(outline, &screens);
        assert_eq!(
            split,
            [
                (1, [180.0, 0.0, 920.0, 420.0]),
                (2, [180.0, -224.0, 920.0, 224.0])
            ]
        );
        let alone = [100.0, 200.0, 600.0, 400.0];
        assert_eq!(pieces(alone, &screens), [(1, alone)]);
        // A window left where an unplugged display was: on none, so it's
        // drawn whole on the nearest (the host hides it instead).
        assert_eq!(pieces([-4000.0, 0.0, 100.0, 100.0], &screens).len(), 1);
    }

    #[test]
    fn pieces_snap_to_each_displays_own_pixels() {
        let screens = three();
        // Across the 1× display and the built-in 2× one.
        let outline = [-300.3, 500.25, 600.4, 200.0];
        let split = pieces(outline, &screens);
        assert_eq!(split.len(), 2);
        let (id, at_1x) = split.iter().find(|(id, _)| *id == 3).copied().unwrap();
        assert_eq!(id, 3);
        assert_eq!(at_1x, [-300.0, 500.0, 300.0, 200.0], "whole points at 1×");
        let (_, at_2x) = split.iter().find(|(id, _)| *id == 1).copied().unwrap();
        assert_eq!(at_2x, [0.0, 500.5, 300.0, 200.0], "half points at 2×");
        assert_eq!(snap([0.3, 0.3, 10.0, 10.0], 2.0), [0.5, 0.5, 10.0, 10.0]);
        assert_eq!(snap([0.3, 0.3, 10.0, 10.0], 1.0), [0.0, 0.0, 10.0, 10.0]);
    }

    #[test]
    fn the_spot_past_the_hud_carries_to_the_other_display() {
        let screens = two();
        // Under the HUD at the top centre of the built-in display.
        let spot = (900.0, 70.0);
        assert_eq!(
            carry(spot, true, &screens[0], &screens[1]),
            (676.0, -1370.0)
        );
        // A HUD at the bottom keeps its distance from the bottom.
        let low = (900.0, 1100.0);
        assert_eq!(carry(low, false, &screens[0], &screens[1]), (676.0, -69.0));
        assert_eq!(carry(spot, true, &screens[0], &screens[0]), spot);
    }
}
