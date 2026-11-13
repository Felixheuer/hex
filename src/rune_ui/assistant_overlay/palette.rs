//! The assistant's colours in Rune: each task's (`TaskStatus.colour`,
//! 0..6) from `assistant::palette`, the single source its cursor, outline,
//! title chip, rail dot, and rows share; the assistant's own hue (the two
//! the screen frame glows in, [`frame`] / [`agent`]); and the three
//! semantic colours a state chip wears ([`done`], [`failed`], [`needs`]).
//!
//! Colour never carries meaning alone (`docs/ux/redesign.md` §3.2): a state
//! always comes with its glyph and its word (`super::look::State`), and a
//! task's dot with its title. The tests below hold the rules: every task
//! colour stands apart from every other *and* from the semantic and agent
//! hues, reads on both desktops, carries ink at 4.5:1, and every chip's
//! words read at 4.5:1 on their tint.

use rune::{Color, oklch};

/// How many task colours there are; `TaskStatus.colour` wraps at this.
pub(crate) const COLOURS: u8 = crate::assistant::palette::COLOURS.len() as u8;

/// How strongly a chip is tinted with its colour, over the card's
/// `bg_base` (`super::look::chip`).
pub(crate) const CHIP_TINT: f32 = 0.14;

/// Task `index`'s colour (wrapping past [`COLOURS`]): its deeper shade
/// over light desktops, its lighter one over dark.
pub(crate) fn task(index: u8, dark: bool) -> Color {
    let colour = crate::assistant::palette::colour(index);
    rune::color::hex(if dark { colour.dark } else { colour.light }).unwrap_or(Color::BLACK)
}

/// Text on a [`task`] colour's solid fill: the kit's black or white,
/// whichever reads better (at least 4.5:1 for every task colour).
pub(crate) fn ink(colour: Color, k: &rune_kit::Kit) -> Color {
    let contrast = |a: Color| contrast(a, colour);
    if contrast(k.c.black) >= contrast(k.c.white) {
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

/// The assistant's own hue (the frame's lead): only for the assistant
/// itself, never a task: the island's glyphs, the desk frame, the Here
/// outline, listening, thinking, working.
pub(crate) fn agent(dark: bool) -> Color {
    frame(dark).0
}

/// Done: green, 150°.
pub(crate) fn done(dark: bool) -> Color {
    if dark {
        oklch(0.78, 0.16, 150.0)
    } else {
        oklch(0.58, 0.15, 150.0)
    }
}

/// Failed: red, 25°.
pub(crate) fn failed(dark: bool) -> Color {
    if dark {
        oklch(0.72, 0.17, 25.0)
    } else {
        oklch(0.56, 0.19, 25.0)
    }
}

/// Needs you: amber, 75–85°.
pub(crate) fn needs(dark: bool) -> Color {
    if dark {
        oklch(0.84, 0.14, 85.0)
    } else {
        oklch(0.7, 0.15, 75.0)
    }
}

/// The words (and glyph) on a chip tinted with `colour`: the same hue,
/// deep enough over light cards and light enough over dark ones to read
/// at 4.5:1 on the tint.
pub(crate) fn chip_ink(colour: Color, dark: bool) -> Color {
    let [l, a, b, _] = colour.to_oklab();
    let (l, chroma) = if dark {
        (l.max(0.86), 0.85)
    } else {
        (l.min(0.45), 1.0)
    };
    Color::from_oklab([l, a * chroma, b * chroma, 1.0])
}

/// A chip's tint as it shows: `colour` at [`CHIP_TINT`] over `backdrop`.
#[cfg_attr(not(test), allow(dead_code))] // The chip contrast test; the renderer blends it itself.
pub(crate) fn tint(colour: Color, backdrop: Color) -> Color {
    backdrop.mix(colour.alpha(1.0), CHIP_TINT).alpha(1.0)
}

/// WCAG contrast of two opaque colours.
pub(crate) fn contrast(a: Color, b: Color) -> f32 {
    let (a, b) = (a.luminance(), b.luminance());
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Perceptual distance (ΔE in Oklab).
    fn apart(a: Color, b: Color) -> f32 {
        let ([l1, a1, b1, _], [l2, a2, b2, _]) = (a.to_oklab(), b.to_oklab());
        ((l1 - l2).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt()
    }

    /// What colour already means on the overlay, by name.
    fn meanings(dark: bool) -> [(&'static str, Color); 5] {
        let (lead, trail) = frame(dark);
        [
            ("done", done(dark)),
            ("failed", failed(dark)),
            ("needs you", needs(dark)),
            ("agent", lead),
            ("agent's second hue", trail),
        ]
    }

    #[test]
    fn every_colour_reads_on_both_desktops_and_carries_readable_ink() {
        for dark in [false, true] {
            let k = rune_theme_opencode::kit(dark);
            let backdrop = k.c.bg_base;
            for index in 0..COLOURS {
                let colour = task(index, dark);
                assert!(
                    contrast(colour, backdrop) >= 3.0,
                    "colour {index} (dark {dark}) fades into the window behind it"
                );
                assert!(
                    contrast(colour, ink(colour, k)) >= 4.5,
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
                    let d = apart(task(a, dark), task(b, dark));
                    assert!(
                        d >= 0.1,
                        "colours {a} and {b} (dark {dark}) look alike: {d}"
                    );
                }
            }
        }
        assert_eq!(task(COLOURS + 1, true), task(1, true));
    }

    #[test]
    fn every_task_colour_stands_apart_from_semantic_and_agent_hues() {
        for dark in [false, true] {
            for index in 0..COLOURS {
                for (name, meaning) in meanings(dark) {
                    let d = apart(task(index, dark), meaning);
                    assert!(
                        d >= 0.1,
                        "colour {index} (dark {dark}) looks like {name}: {d}"
                    );
                }
            }
            // The semantic colours stand apart from each other, too.
            let all = meanings(dark);
            for (i, (a, ca)) in all.iter().enumerate() {
                for (b, cb) in &all[i + 1..] {
                    assert!(
                        apart(*ca, *cb) >= 0.1,
                        "{a} and {b} (dark {dark}) look alike"
                    );
                }
            }
        }
    }

    #[test]
    fn chip_words_have_4_5_contrast() {
        for dark in [false, true] {
            let k = rune_theme_opencode::kit(dark);
            let card = k.c.bg_base.alpha(1.0);
            let colours = meanings(dark)
                .map(|(_, colour)| colour)
                .into_iter()
                .chain((0..COLOURS).map(|index| task(index, dark)));
            for colour in colours {
                let words = chip_ink(colour, dark);
                let c = contrast(words, tint(colour, card));
                assert!(
                    c >= 4.5,
                    "a chip in {} (dark {dark}) reads at {c}",
                    colour.hex()
                );
            }
        }
    }
}
