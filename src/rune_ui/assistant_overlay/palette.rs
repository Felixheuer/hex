//! The assistant's colours in Rune: each task's (`TaskStatus.colour`,
//! 0..8) from `assistant::palette`, the single source its cursor, outline,
//! title chip, orb dot, and rows share, and the two hues the screen frame
//! glows in.

use rune::{Color, oklch};

/// How many task colours there are; `TaskStatus.colour` wraps at this.
pub(crate) const COLOURS: u8 = crate::assistant::palette::COLOURS.len() as u8;

/// Task `index`'s colour (wrapping past [`COLOURS`]): its deeper shade
/// over light desktops, its lighter one over dark.
pub(crate) fn task(index: u8, dark: bool) -> Color {
    let colour = crate::assistant::palette::colour(index);
    rune::color::hex(if dark { colour.dark } else { colour.light }).unwrap_or(Color::BLACK)
}

/// Text on a [`task`] colour's solid fill: the kit's black or white,
/// whichever reads better.
pub(crate) fn ink(colour: Color, k: &rune_kit::Kit) -> Color {
    if colour.luminance() > 0.179 {
        k.c.black
    } else {
        k.c.white
    }
}

/// The screen frame's two hues: one glows from the top and left edges, the
/// other from the bottom and right, meeting in the corners.
pub(crate) fn frame(dark: bool) -> (Color, Color) {
    if dark {
        (oklch(0.72, 0.16, 255.0), oklch(0.7, 0.18, 305.0))
    } else {
        (oklch(0.62, 0.2, 258.0), oklch(0.6, 0.22, 305.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Color, b: Color) -> f32 {
        let (a, b) = (a.luminance(), b.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_colour_reads_on_both_desktops_and_carries_readable_ink() {
        for dark in [false, true] {
            let k = rune_theme_opencode::kit(dark);
            let backdrop = k.c.bg_base;
            for index in 0..COLOURS {
                let colour = task(index, dark);
                assert!(
                    contrast(colour, backdrop) >= 2.2,
                    "colour {index} (dark {dark}) fades into the window behind it"
                );
                assert!(
                    contrast(colour, ink(colour, k)) >= 3.0,
                    "colour {index} (dark {dark}) has unreadable ink"
                );
            }
        }
    }

    #[test]
    fn every_colour_stands_apart_from_every_other_and_wraps() {
        for dark in [false, true] {
            for a in 0..COLOURS {
                for b in a + 1..COLOURS {
                    // Perceptual distance (ΔE in Oklab).
                    let ([l1, a1, b1, _], [l2, a2, b2, _]) =
                        (task(a, dark).to_oklab(), task(b, dark).to_oklab());
                    let apart = ((l1 - l2).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt();
                    assert!(
                        apart >= 0.1,
                        "colours {a} and {b} (dark {dark}) look alike: {apart}"
                    );
                }
            }
        }
        assert_eq!(task(9, true), task(1, true));
    }
}
