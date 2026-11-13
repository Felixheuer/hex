//! The task colours, the single source for everything that tells tasks
//! apart: each running task's agent cursor (agent-desktop's
//! `cursor-overlay`), its window outline and title chip, its dot in the
//! orb, and its rows in the list and Chats. `TaskStatus.colour` indexes
//! [`COLOURS`] (wrapping).
//!
//! Six hues (`docs/ux/redesign.md` §3.2: azure, amber, teal, rose, lime,
//! clay), at least 0.1 apart (ΔE in Oklab) from each other *and* from what
//! colour already means on the overlay: the done green, the failed red,
//! the needs-you amber, and the assistant's own two frame hues (periwinkle
//! and violet). So no task looks like a state: red, green, blue, and violet
//! left the set. Each has a deeper shade over light desktops and a lighter
//! one over dark desktops, both readable with black or white text on top
//! (`rune_ui::assistant_overlay::palette`'s tests hold all of it).
//!
//! OWNER: overlays. The brain reads [`cursor_flags`] for each task's cursor.

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
pub const COLOURS: [TaskColour; 6] = [
    TaskColour {
        name: "azure",
        light: "#068BBD",
        dark: "#7FCBFF",
    },
    TaskColour {
        name: "amber",
        light: "#C26300",
        dark: "#FF9101",
    },
    TaskColour {
        name: "teal",
        light: "#007F79",
        dark: "#04CAC7",
    },
    TaskColour {
        name: "rose",
        light: "#CB409E",
        dark: "#FB79E3",
    },
    TaskColour {
        name: "lime",
        light: "#898500",
        dark: "#B6B900",
    },
    TaskColour {
        name: "clay",
        light: "#97523A",
        dark: "#C78550",
    },
];

/// Task `index`'s colour (wrapping past the sixth).
pub fn colour(index: u8) -> &'static TaskColour {
    &COLOURS[usize::from(index) % COLOURS.len()]
}

/// The flags agent-desktop's `cursor-overlay enable` takes for task
/// `index`'s cursor: its accent, a lighter fill, and a white rim, so the
/// cursor matches the task's outline and dot. Add `--label` and `--image`
/// (`rune_ui::assistant_overlay::cursor_image`) as wanted.
pub fn cursor_flags(index: u8) -> Vec<String> {
    let colour = colour(index);
    [
        "--accent",
        colour.light,
        "--fill",
        colour.dark,
        "--rim",
        "#FFFFFF",
    ]
    .map(str::to_owned)
    .to_vec()
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
        assert_eq!(colour(7), colour(1));
        assert_eq!(cursor_flags(0)[1], "#068BBD");
    }
}
