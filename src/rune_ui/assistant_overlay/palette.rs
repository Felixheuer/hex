//! The assistant's task colours: one per running task (`TaskStatus.colour`,
//! 0..8), shared by its cursor, window outline, and stack row, and the two
//! hues the screen frame glows in. The kit has no categorical palette, so
//! this is the one place these colours are authored, in OKLCH at a
//! lightness for each appearance: darker and more saturated over light
//! desktops, lighter over dark ones, so every hue reads on both.

use rune::{Color, oklch};

/// How many task colours there are; `TaskStatus.colour` wraps at this.
pub(crate) const COLOURS: u8 = 8;

/// The hues in the order tasks take them: each next one far from the ones
/// before, so the first few tasks never look alike (blue, orange, green,
/// violet, rose, teal, amber, indigo).
const HUES: [f32; COLOURS as usize] = [252.0, 48.0, 150.0, 302.0, 0.0, 195.0, 85.0, 275.0];

/// Task `index`'s colour (wrapping past [`COLOURS`]).
pub(crate) fn task(index: u8, dark: bool) -> Color {
    let hue = HUES[usize::from(index % COLOURS)];
    // Amber and green are light by nature; they come down a little on light
    // desktops to keep their edge against white windows.
    let warm = matches!(index % COLOURS, 2 | 6);
    match (dark, warm) {
        (true, _) => oklch(0.74, 0.15, hue),
        (false, true) => oklch(0.6, 0.16, hue),
        (false, false) => oklch(0.6, 0.19, hue),
    }
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
    fn neighbouring_colours_are_far_apart_and_wrap() {
        for index in 0..COLOURS - 1 {
            let (a, b) = (HUES[usize::from(index)], HUES[usize::from(index + 1)]);
            let apart = (a - b).abs().min(360.0 - (a - b).abs());
            assert!(
                apart >= 45.0,
                "hues {index} and {} are {apart}° apart",
                index + 1
            );
        }
        assert_eq!(task(9, true), task(1, true));
    }
}
