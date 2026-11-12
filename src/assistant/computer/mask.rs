//! Which parts of a screenshot are blacked out where the OS can't leave
//! windows out of the capture itself (Windows, X11): the parts of Hex's
//! own and blocked apps' windows that show, with what lies in front of them
//! cut away.

/// `[x, y, width, height]` in the screenshot's global coordinates.
pub type Rect = [f64; 4];

/// A window on screen, as the mask sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub rect: Rect,
    /// Never part of the screenshot (Hex's own, or a blocked app's).
    pub hidden: bool,
    /// An ordinary app window that hides what's behind it. Click-through
    /// overlays and tool windows don't count: whatever they cover is still
    /// blacked out.
    pub opaque: bool,
}

/// The rectangles to black out, for `layers` listed front to back: each
/// hidden window less the opaque windows in front of it.
pub fn hidden_parts(layers: &[Layer]) -> Vec<Rect> {
    let mut covers: Vec<Rect> = Vec::new();
    let mut hidden = Vec::new();
    for layer in layers {
        if empty(layer.rect) {
            continue;
        }
        if layer.hidden {
            hidden.extend(subtract(layer.rect, &covers));
        } else if layer.opaque {
            covers.push(layer.rect);
        }
    }
    hidden
}

fn empty(rect: Rect) -> bool {
    rect[2] <= 0.0 || rect[3] <= 0.0
}

/// `rect` less every one of `covers`, as rectangles that don't overlap.
pub fn subtract(rect: Rect, covers: &[Rect]) -> Vec<Rect> {
    let mut pieces = vec![rect];
    for cover in covers {
        pieces = pieces
            .into_iter()
            .flat_map(|piece| minus(piece, *cover))
            .collect();
        if pieces.is_empty() {
            break;
        }
    }
    pieces
}

/// `rect` less `cover`: up to four strips around it.
fn minus(rect: Rect, cover: Rect) -> Vec<Rect> {
    let [x, y, w, h] = rect;
    let (right, bottom) = (x + w, y + h);
    let [cx, cy, cw, ch] = cover;
    let (cright, cbottom) = (cx + cw, cy + ch);
    if cx >= right || cright <= x || cy >= bottom || cbottom <= y {
        return vec![rect];
    }
    let mut strips = Vec::new();
    // Above and below the cover, the rect's whole width.
    if cy > y {
        strips.push([x, y, w, cy - y]);
    }
    if cbottom < bottom {
        strips.push([x, cbottom, w, bottom - cbottom]);
    }
    // Left and right of it, between those.
    let top = y.max(cy);
    let low = bottom.min(cbottom);
    if cx > x {
        strips.push([x, top, cx - x, low - top]);
    }
    if cright < right {
        strips.push([cright, top, right - cright, low - top]);
    }
    strips.retain(|strip| !empty(*strip));
    strips
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(rects: &[Rect]) -> f64 {
        rects.iter().map(|rect| rect[2] * rect[3]).sum()
    }

    fn layer(rect: Rect, hidden: bool, opaque: bool) -> Layer {
        Layer {
            rect,
            hidden,
            opaque,
        }
    }

    #[test]
    fn a_cover_cuts_strips_out_of_a_rect() {
        let rect = [0.0, 0.0, 100.0, 100.0];
        assert_eq!(subtract(rect, &[]), [rect]);
        assert!(subtract(rect, &[[-10.0, -10.0, 200.0, 200.0]]).is_empty());
        assert_eq!(subtract(rect, &[[200.0, 0.0, 10.0, 10.0]]), [rect]);
        // A hole in the middle: four strips around it.
        let holed = subtract(rect, &[[25.0, 25.0, 50.0, 50.0]]);
        assert_eq!(holed.len(), 4);
        assert_eq!(area(&holed), 10_000.0 - 2_500.0);
        // The right half covered: the left half stays.
        assert_eq!(
            subtract(rect, &[[50.0, -5.0, 100.0, 200.0]]),
            [[0.0, 0.0, 50.0, 100.0]]
        );
        // Two covers overlapping each other: the right 40 stay, less the
        // second cover's 10 × 50 that reaches past the first.
        let left = subtract(rect, &[[0.0, 0.0, 60.0, 100.0], [40.0, 0.0, 30.0, 50.0]]);
        assert_eq!(area(&left), 40.0 * 100.0 - 10.0 * 50.0);
    }

    /// A blocked app behind the window the person works in shows only
    /// where it peeks out; one in front of it shows whole; overlays in
    /// front don't hide it.
    #[test]
    fn only_what_shows_of_a_hidden_window_is_blacked_out() {
        let editor = layer([0.0, 0.0, 800.0, 600.0], false, true);
        let behind = layer([600.0, 400.0, 400.0, 400.0], true, false);
        let parts = hidden_parts(&[editor, behind]);
        assert_eq!(area(&parts), 400.0 * 400.0 - 200.0 * 200.0);
        let in_front = hidden_parts(&[behind, editor]);
        assert_eq!(in_front, [behind.rect]);
        let overlay = layer([0.0, 0.0, 2000.0, 2000.0], false, false);
        assert_eq!(hidden_parts(&[overlay, behind]), [behind.rect]);
        let minimized = layer([0.0, 0.0, 0.0, 0.0], true, true);
        assert!(hidden_parts(&[minimized]).is_empty());
    }
}
