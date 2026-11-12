//! TEMPORARY SHIM: remove at integration.
//!
//! Stand-ins for the guard engineer's `computer::displays()` (here
//! [`displays`], with the same fields) and the setting
//! `assistant.agent_display: Option<String>` (here [`agent_display`] and
//! [`set_agent_display`], kept in memory only, so the choice doesn't
//! survive a relaunch until the setting lands). At integration: delete this
//! file and `mod display_shim;`, read `display_shim::displays()` as
//! `crate::assistant::computer::displays()`, `display_shim::Display` as
//! theirs, and the two setting functions as
//! `ui.settings.assistant.agent_display` read and written through
//! `ui.update`.

use std::sync::Mutex;

/// One connected display, as the agents' display picker shows it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::rune_ui) struct Display {
    pub(in crate::rune_ui) id: String,
    pub(in crate::rune_ui) name: String,
    /// Global points: x, y, width, height.
    pub(in crate::rune_ui) frame: (f64, f64, f64, f64),
    pub(in crate::rune_ui) scale: f64,
    pub(in crate::rune_ui) is_primary: bool,
    /// The pointer is on it: the one you're working on.
    pub(in crate::rune_ui) has_focus: bool,
}

/// The connected displays, the primary first.
pub(in crate::rune_ui) fn displays() -> Vec<Display> {
    #[cfg(target_os = "macos")]
    {
        use scrawl_capture::screens;
        let pointer = screens::pointer().unwrap_or_default();
        let found = screens::displays();
        let mut displays: Vec<Display> = found
            .iter()
            .enumerate()
            .map(|(index, screen)| {
                let [x, y, w, h] = screen.frame;
                // The primary display's top-left is the global origin.
                let is_primary = x == 0.0 && y == 0.0;
                Display {
                    id: screen.id.to_string(),
                    name: if is_primary {
                        "Main display".into()
                    } else {
                        format!("Display {}", index + 1)
                    },
                    frame: (x, y, w, h),
                    scale: screen.scale,
                    is_primary,
                    has_focus: pointer[0] >= x
                        && pointer[0] < x + w
                        && pointer[1] >= y
                        && pointer[1] < y + h,
                }
            })
            .collect();
        displays.sort_by_key(|display| !display.is_primary);
        displays
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

static AGENT_DISPLAY: Mutex<Option<String>> = Mutex::new(None);

/// The display agents work on; `None` is Auto (the one you're not on).
pub(in crate::rune_ui) fn agent_display() -> Option<String> {
    AGENT_DISPLAY
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

pub(in crate::rune_ui) fn set_agent_display(display: Option<String>) {
    *AGENT_DISPLAY
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = display;
}

/// Two displays side by side, the pointer on the main one, for scenes.
pub(in crate::rune_ui) fn fixture() -> Vec<Display> {
    vec![
        Display {
            id: "1".into(),
            name: "Built-in Retina Display".into(),
            frame: (0.0, 0.0, 1512.0, 982.0),
            scale: 2.0,
            is_primary: true,
            has_focus: true,
        },
        Display {
            id: "2".into(),
            name: "Studio Display".into(),
            frame: (1512.0, -300.0, 2560.0, 1440.0),
            scale: 2.0,
            is_primary: false,
            has_focus: false,
        },
    ]
}
