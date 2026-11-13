//! The task colours, the single source for everything that tells tasks
//! apart: each running task's window outline and title chip, its dot in the
//! orb, and its rows in the list and Chats. `TaskStatus.colour` indexes
//! [`COLOURS`] (wrapping).
//!
//! Eight hues after Okabe and Ito's colour-blind-safe set, tuned to sit well
//! together: neighbours differ in hue and lightness, so the first few tasks
//! never look alike, and the set stays apart under the common colour-vision
//! deficiencies (blue, orange, teal, magenta, olive, violet, red, brown).
//! Each has a deeper shade over light desktops and a lighter
//! one over dark desktops, both readable with black or white text on top.
//!
//! OWNER: overlays.

/// One task colour, as `#RRGGBB`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskColour {
    pub name: &'static str,
    /// Over light desktops (and the cursor's accent).
    pub light: &'static str,
    /// Over dark desktops.
    pub dark: &'static str,
}

/// The colours in the order tasks take them.
pub const COLOURS: [TaskColour; 8] = [
    TaskColour {
        name: "blue",
        light: "#2B6FE0",
        dark: "#6FA8FF",
    },
    TaskColour {
        name: "orange",
        light: "#D97500",
        dark: "#FFA94A",
    },
    TaskColour {
        name: "teal",
        light: "#00916E",
        dark: "#3CCFA6",
    },
    TaskColour {
        name: "magenta",
        light: "#C23E92",
        dark: "#F287CC",
    },
    TaskColour {
        name: "olive",
        light: "#8A8A00",
        dark: "#D9D94A",
    },
    TaskColour {
        name: "violet",
        light: "#8E44D8",
        dark: "#C08CFF",
    },
    TaskColour {
        name: "red",
        light: "#D8342C",
        dark: "#FF7A6E",
    },
    TaskColour {
        name: "brown",
        light: "#8B5E34",
        dark: "#C99B6E",
    },
];

/// Task `index`'s colour (wrapping past the eighth).
pub fn colour(index: u8) -> &'static TaskColour {
    &COLOURS[usize::from(index) % COLOURS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_well_formed_and_wrap() {
        for colour in COLOURS {
            for hex in [colour.light, colour.dark] {
                assert!(
                    hex.len() == 7
                        && hex.starts_with('#')
                        && hex[1..].chars().all(|c| c.is_ascii_hexdigit()),
                    "{hex}"
                );
            }
        }
        assert_eq!(colour(9), colour(1));
        assert_eq!(colour(0).light, "#2B6FE0");
    }
}
