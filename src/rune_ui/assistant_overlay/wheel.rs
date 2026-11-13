//! The wheel and the trackpad on the overlay panels.
//!
//! Rune's overlay host hands its runtime the pointer, presses, and keys,
//! but drops `WindowEvent::MouseWheel` (`overlay/any_host.rs` has no arm
//! for it), so no scroll viewport in a panel ever moved: nine browsers in
//! a column showed the first few and the rest stayed out of reach. The
//! host therefore watches scroll events itself (`host::wheel`, an AppKit
//! local monitor, which sees them whether or not the panel is key) and
//! routes each here: every scroll viewport in a panel registers with
//! [`scroller`] (where it's drawn, how far it's scrolled, how far it can),
//! [`route`] picks the innermost one under the pointer that can move on
//! the gesture's axis, and the host moves it with [`Scroll::apply`]. A
//! horizontal strip takes a trackpad's sideways deltas, and a mouse
//! wheel's when nothing around it scrolls up and down. A picture the
//! person drives takes the wheel itself ([`sink`]), for its page.
//!
//! The registry lives on the thread that draws the panels (the main
//! thread in the app, each test's own in tests).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rune::El;

/// The panel a viewport is in.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Surface {
    /// The rail: job rows, the desk, the live overview, the list.
    Orb,
    /// The live wall.
    Wall,
    /// The stage under the island.
    Stage,
    /// The enlarged live card.
    Enlarged,
}

/// Which way a viewport scrolls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Axis {
    Y,
    X,
}

/// What takes the wheel over a picture the person drives: the point
/// inside it and the deltas (content moving with the fingers).
pub(crate) type Sink = Rc<dyn Fn((f32, f32), (f32, f32))>;

#[derive(Clone)]
struct Viewport {
    axis: Axis,
    /// Where the last frame drew it, in panel points; `None` before that.
    rect: Option<[f32; 4]>,
    offset: f32,
    max: f32,
    /// In the panel's latest view (a folded list leaves its viewport
    /// behind until the view runs without it).
    alive: bool,
    sink: Option<Sink>,
}

thread_local! {
    static VIEWPORTS: RefCell<HashMap<(Surface, String), Viewport>> = RefCell::new(HashMap::new());
}

/// A panel's view starts: viewports it doesn't register again are gone.
pub(crate) fn begin(surface: Surface) {
    VIEWPORTS.with(|all| {
        for ((at, _), viewport) in all.borrow_mut().iter_mut() {
            if *at == surface {
                viewport.alive = false;
            }
        }
    });
}

fn register(surface: Surface, key: &str, axis: Axis, sink: Option<Sink>) {
    VIEWPORTS.with(|all| {
        let mut all = all.borrow_mut();
        let viewport = all
            .entry((surface, key.to_owned()))
            .or_insert_with(|| Viewport {
                axis,
                rect: None,
                offset: 0.0,
                max: 0.0,
                alive: true,
                sink: None,
            });
        viewport.axis = axis;
        viewport.alive = true;
        viewport.sink = sink;
    });
}

fn with_viewport(surface: Surface, key: &str, f: impl FnOnce(&mut Viewport)) {
    VIEWPORTS.with(|all| {
        if let Some(viewport) = all.borrow_mut().get_mut(&(surface, key.to_owned())) {
            f(viewport);
        }
    });
}

/// `el`, a scroll viewport keyed `key` in `surface` that scrolls on
/// `axis`, reachable by the wheel and the trackpad.
pub(crate) fn scroller<S: 'static>(el: El, surface: Surface, key: &str, axis: Axis) -> El {
    register(surface, key, axis, None);
    let (at, drawn) = (key.to_owned(), key.to_owned());
    el.on_scroll_offset(move |pos: rune::ScrollPos| {
        let i = if axis == Axis::X { 0 } else { 1 };
        with_viewport(surface, &at, |viewport| {
            viewport.offset = pos.offset[i];
            viewport.max = pos.max[i];
        });
    })
    .on_drawn_layout_tx(move |_: &mut S, rect, tx| {
        with_viewport(surface, &drawn, |viewport| viewport.rect = Some(rect));
        tx.live_only();
    })
}

/// `el` (keyed `key` in `surface`) takes the wheel itself: `sink` hears
/// every delta over it, before any viewport around it.
pub(crate) fn sink<S: 'static>(el: El, surface: Surface, key: &str, sink: Sink) -> El {
    register(surface, key, Axis::Y, Some(sink));
    let drawn = key.to_owned();
    el.on_drawn_layout_tx(move |_: &mut S, rect, tx| {
        with_viewport(surface, &drawn, |viewport| viewport.rect = Some(rect));
        tx.live_only();
    })
}

/// A move for the host to make: viewport `key` to `to` on `axis`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Scroll {
    pub(crate) key: String,
    pub(crate) axis: Axis,
    pub(crate) to: f32,
}

impl Scroll {
    /// Moves the viewport, at once (the deltas come as fast as the fingers
    /// move, momentum included).
    pub(crate) fn apply(&self, tx: &mut rune::Tx) {
        match self.axis {
            Axis::Y => tx.scroll_to_y(self.key.clone(), self.to),
            Axis::X => tx.scroll_to_x(self.key.clone(), self.to),
        }
    }
}

fn contains([x, y, w, h]: [f32; 4], (px, py): (f32, f32)) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

/// A wheel or trackpad delta `(dx, dy)` in points (positive moves content
/// right and down, with the fingers) at `point` in `surface`: the move it
/// makes, if any viewport there can take it. A picture taking the wheel
/// hears it instead (and `None` comes back).
pub(crate) fn route(surface: Surface, point: (f32, f32), (dx, dy): (f32, f32)) -> Option<Scroll> {
    if dx == 0.0 && dy == 0.0 {
        return None;
    }
    let sink = VIEWPORTS.with(|all| {
        all.borrow()
            .iter()
            .filter(|((at, _), viewport)| *at == surface && viewport.alive)
            .filter_map(|(_, viewport)| {
                let rect = viewport.rect?;
                let sink = viewport.sink.clone()?;
                contains(rect, point).then_some((rect, sink))
            })
            .next()
    });
    if let Some((rect, sink)) = sink {
        sink((point.0 - rect[0], point.1 - rect[1]), (dx, dy));
        return None;
    }
    VIEWPORTS.with(|all| {
        let mut all = all.borrow_mut();
        // The innermost (smallest) viewport under the pointer that can
        // still move on `axis` by `delta`.
        let pick = |all: &HashMap<(Surface, String), Viewport>, axis: Axis, delta: f32| {
            all.iter()
                .filter(|((at, _), viewport)| {
                    *at == surface
                        && viewport.alive
                        && viewport.sink.is_none()
                        && viewport.axis == axis
                        && viewport.max > 0.5
                        && viewport.rect.is_some_and(|rect| contains(rect, point))
                        && if delta > 0.0 {
                            viewport.offset > 0.0
                        } else {
                            viewport.offset < viewport.max
                        }
                })
                .min_by(|(_, a), (_, b)| {
                    let area = |v: &Viewport| v.rect.map_or(f32::MAX, |r| r[2] * r[3]);
                    area(a).total_cmp(&area(b))
                })
                .map(|((_, key), _)| key.clone())
        };
        let sideways = dx.abs() > dy.abs();
        let (axis, delta, key) = if sideways {
            (Axis::X, dx, pick(&all, Axis::X, dx))
        } else {
            match pick(&all, Axis::Y, dy) {
                Some(key) => (Axis::Y, dy, Some(key)),
                // A mouse wheel over a strip that only scrolls sideways.
                None => (Axis::X, dy, pick(&all, Axis::X, dy)),
            }
        };
        let key = key?;
        let viewport = all.get_mut(&(surface, key.clone()))?;
        let to = (viewport.offset - delta).clamp(0.0, viewport.max);
        // Deltas come faster than frames: the next one starts here.
        viewport.offset = to;
        Some(Scroll { key, axis, to })
    })
}

/// Every viewport of `surface` as the registry has it (tests).
#[cfg(test)]
pub(crate) fn dump(surface: Surface) -> Vec<String> {
    VIEWPORTS.with(|all| {
        all.borrow()
            .iter()
            .filter(|((at, _), _)| *at == surface)
            .map(|((_, key), v)| {
                format!(
                    "{key} alive={} rect={:?} offset={} max={}",
                    v.alive, v.rect, v.offset, v.max
                )
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(surface: Surface, key: &str, axis: Axis, rect: [f32; 4], offset: f32, max: f32) {
        register(surface, key, axis, None);
        with_viewport(surface, key, |viewport| {
            viewport.rect = Some(rect);
            viewport.offset = offset;
            viewport.max = max;
        });
    }

    #[test]
    fn the_innermost_viewport_that_can_move_takes_the_delta() {
        begin(Surface::Wall);
        put(
            Surface::Wall,
            "outer",
            Axis::Y,
            [0.0, 0.0, 400.0, 400.0],
            0.0,
            300.0,
        );
        put(
            Surface::Wall,
            "strip",
            Axis::X,
            [0.0, 0.0, 300.0, 100.0],
            0.0,
            500.0,
        );
        // Down over the strip: the column around it.
        let down = route(Surface::Wall, (10.0, 10.0), (0.0, -40.0)).unwrap();
        assert_eq!(
            (down.key.as_str(), down.axis, down.to),
            ("outer", Axis::Y, 40.0)
        );
        // Sideways on a trackpad: the strip.
        let side = route(Surface::Wall, (10.0, 10.0), (-30.0, -2.0)).unwrap();
        assert_eq!(
            (side.key.as_str(), side.axis, side.to),
            ("strip", Axis::X, 30.0)
        );
        // Several deltas before a frame add up.
        assert_eq!(
            route(Surface::Wall, (10.0, 10.0), (-30.0, 0.0)).unwrap().to,
            60.0
        );
        // Back past the start stops at it; outside every viewport nothing.
        assert_eq!(
            route(Surface::Wall, (10.0, 200.0), (0.0, 400.0))
                .unwrap()
                .to,
            0.0
        );
        assert!(
            route(Surface::Wall, (10.0, 200.0), (0.0, 40.0)).is_none(),
            "at the top"
        );
        assert!(route(Surface::Wall, (900.0, 10.0), (0.0, -40.0)).is_none());
        // Another panel's viewports never answer.
        assert!(route(Surface::Stage, (10.0, 10.0), (0.0, -40.0)).is_none());
    }

    #[test]
    fn a_wheel_over_a_lone_strip_scrolls_it_sideways() {
        begin(Surface::Orb);
        put(
            Surface::Orb,
            "film",
            Axis::X,
            [20.0, 20.0, 300.0, 100.0],
            0.0,
            900.0,
        );
        let moved = route(Surface::Orb, (40.0, 40.0), (0.0, -120.0)).unwrap();
        assert_eq!((moved.axis, moved.to), (Axis::X, 120.0));
    }

    #[test]
    fn a_viewport_gone_from_the_view_takes_nothing() {
        begin(Surface::Stage);
        put(
            Surface::Stage,
            "answer",
            Axis::Y,
            [0.0, 0.0, 400.0, 400.0],
            0.0,
            300.0,
        );
        assert!(route(Surface::Stage, (10.0, 10.0), (0.0, -40.0)).is_some());
        begin(Surface::Stage);
        assert!(route(Surface::Stage, (10.0, 10.0), (0.0, -40.0)).is_none());
    }

    #[test]
    fn a_driven_picture_takes_the_wheel_itself() {
        let heard: Rc<RefCell<Vec<(f32, f32)>>> = Rc::default();
        let into = heard.clone();
        begin(Surface::Enlarged);
        register(
            Surface::Enlarged,
            "live-picture",
            Axis::Y,
            Some(Rc::new(move |_, delta| into.borrow_mut().push(delta))),
        );
        with_viewport(Surface::Enlarged, "live-picture", |viewport| {
            viewport.rect = Some([10.0, 10.0, 100.0, 100.0]);
        });
        assert!(route(Surface::Enlarged, (50.0, 50.0), (0.0, -12.0)).is_none());
        assert_eq!(heard.borrow().as_slice(), [(0.0, -12.0)]);
    }
}
