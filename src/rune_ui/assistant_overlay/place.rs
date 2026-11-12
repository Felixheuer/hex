//! Where the orb sits: beside the HUD by default, past where the Here
//! bubble hangs, at the HUD's edge of the pointer's display; wherever the
//! person dragged it on that display once they have, remembered per display
//! across launches (`<support>/assistant/orb.json`). Which way bubbles and
//! the list unfold follows from where it sits, so they always grow into the
//! display. Plain geometry here; the host reads the screens.

use std::collections::HashMap;
use std::sync::Mutex;

use super::orb::{self, Grow};

/// A display's visible area (below the menu bar, beside the Dock) in
/// global points, y down: x, y, width, height.
pub(crate) type Area = [f64; 4];

/// From the HUD's centre to the orb's: clear of the Here bubble, which
/// hangs centred under the HUD and is 440 points wide.
const BESIDE_HUD: f64 = 268.0;
/// From the display's edge to the orb's centre.
const EDGE: f64 = 24.0;
/// The orb's core centre from the panel's corner on its side.
const CORE_IN: (f64, f64) = (
    orb::INSET as f64 + 15.0,
    orb::INSET as f64 + orb::ORB_H as f64 / 2.0,
);

/// The orb's default centre on a display: beside the HUD (`hud`: its centre
/// x and whether it's at the top), else beside the top centre.
pub(crate) fn default_anchor(visible: Area, hud: Option<(f64, bool)>) -> (f64, f64) {
    let [x, y, w, h] = visible;
    let (centre, top) = hud.unwrap_or((x + w / 2.0, true));
    let ay = if top { y + EDGE } else { y + h - EDGE };
    clamp((centre + BESIDE_HUD, ay), visible)
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

/// Where the person dragged the orb on each display: its centre from the
/// visible area's top-left, by display id.
static DRAGGED: Mutex<Option<HashMap<u32, (f64, f64)>>> = Mutex::new(None);

fn with_dragged<T>(f: impl FnOnce(&mut HashMap<u32, (f64, f64)>) -> T) -> T {
    let mut dragged = DRAGGED.lock().unwrap_or_else(|error| error.into_inner());
    f(dragged.get_or_insert_with(load))
}

/// The orb's centre on display `id`: where it was dragged, else beside the
/// HUD.
pub(crate) fn anchor(id: Option<u32>, visible: Area, hud: Option<(f64, bool)>) -> (f64, f64) {
    let dragged = id.and_then(|id| with_dragged(|dragged| dragged.get(&id).copied()));
    match dragged {
        Some((dx, dy)) => clamp((visible[0] + dx, visible[1] + dy), visible),
        None => default_anchor(visible, hud),
    }
}

/// Remembers that the orb was dropped at `anchor` on display `id`.
pub(crate) fn remember(id: u32, visible: Area, anchor: (f64, f64)) {
    let (ax, ay) = clamp(anchor, visible);
    let offset = (ax - visible[0], ay - visible[1]);
    let saved = with_dragged(|dragged| {
        dragged.insert(id, offset);
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

fn load() -> HashMap<u32, (f64, f64)> {
    file()
        .and_then(|path| std::fs::read(path).ok())
        .map(|bytes| parse(&bytes))
        .unwrap_or_default()
}

fn parse(bytes: &[u8]) -> HashMap<u32, (f64, f64)> {
    serde_json::from_slice::<HashMap<String, [f64; 2]>>(bytes)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, [x, y])| {
            (x.is_finite() && y.is_finite()).then_some((id.parse().ok()?, (x, y)))
        })
        .collect()
}

fn save(dragged: &HashMap<u32, (f64, f64)>) {
    let Some(path) = file() else {
        return;
    };
    let json: HashMap<String, [f64; 2]> = dragged
        .iter()
        .map(|(id, (x, y))| (id.to_string(), [*x, *y]))
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
    fn the_orb_sits_beside_the_hud_clear_of_the_here_bubble() {
        let (ax, ay) = default_anchor(VISIBLE, Some((756.0, true)));
        assert!(ax - 756.0 > 220.0, "past the bubble's half width: {ax}");
        assert_eq!(ay, 33.0 + EDGE);
        assert_eq!(grow((ax, ay), VISIBLE), Grow::default());
        let (_, low) = default_anchor(VISIBLE, Some((756.0, false)));
        assert_eq!(low, 33.0 + 879.0 - EDGE, "with the HUD at the bottom");
        assert!(!grow((ax, low), VISIBLE).down, "unfolds upward there");
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
        let parsed = parse(br#"{"1": [900.5, 40], "7": [10, 20]}"#);
        assert_eq!(parsed.get(&1), Some(&(900.5, 40.0)));
        assert_eq!(parsed.get(&7), Some(&(10.0, 20.0)));
    }
}
