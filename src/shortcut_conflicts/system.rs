//! macOS's own users of a shortcut's keys: its keyboard shortcuts
//! (`com.apple.symbolichotkeys`), the 🌐 key's action, Apple Dictation's
//! shortcut, and Siri's.
//!
//! macOS matches its symbolic hot keys in the WindowServer, before any
//! session tap sees the key, so while one is on with Hex's chord Hex never
//! sees the press. They live in `AppleSymbolicHotKeys`, one entry per ID
//! (`{ enabled, value: { parameters: (character, key code, modifiers) } }`);
//! an ID with no entry is on with its default chord. Apple Dictation's and
//! Siri's own shortcuts (press a key twice, hold ⌘ Space) are saved in forms
//! Hex doesn't read, so while either is on a shortcut on those keys is a
//! possible conflict.

use std::collections::HashMap;

use serde_json::Value;

use super::{Certainty, Conflict, Fix, Lone, Source, keys, plist};
use crate::app_settings::HotkeyBinding;

const SHIFT: u64 = 0x2_0000;
const CONTROL: u64 = 0x4_0000;
const OPTION: u64 = 0x8_0000;
const COMMAND: u64 = 0x10_0000;
const MODIFIERS: u64 = SHIFT | CONTROL | OPTION | COMMAND;

/// The key code a symbolic hot key saves when it names no key.
const NO_KEY: i64 = 0xFFFF;
const SPACE: u16 = 49;

/// One symbolic hot key as `AppleSymbolicHotKeys` holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) enabled: bool,
    /// The key code and the modifier flags (CGEventFlags' bits), when the
    /// entry names them.
    pub(crate) chord: Option<(u16, u64)>,
}

/// macOS's keyboard shortcuts Hex names, with the default chord of those on
/// by default. Every other saved one that's on is checked too, as "a macOS
/// keyboard shortcut".
/// A key code and modifier flags.
type Chord = (u16, u64);

const KNOWN: &[(u32, &str, Option<Chord>)] = &[
    (7, "Move focus to the menu bar", None),
    (8, "Move focus to the Dock", None),
    (9, "Move focus to the active or next window", None),
    (10, "Move focus to the window toolbar", None),
    (11, "Move focus to the floating window", None),
    (12, "Turn keyboard access on or off", None),
    (13, "Change the way Tab moves focus", None),
    (27, "Move focus to next window", None),
    (
        28,
        "Save picture of screen as a file",
        Some((20, COMMAND | SHIFT)),
    ),
    (
        29,
        "Copy picture of screen to the clipboard",
        Some((20, CONTROL | COMMAND | SHIFT)),
    ),
    (
        30,
        "Save picture of selected area as a file",
        Some((21, COMMAND | SHIFT)),
    ),
    (
        31,
        "Copy picture of selected area to the clipboard",
        Some((21, CONTROL | COMMAND | SHIFT)),
    ),
    (32, "Mission Control", Some((126, CONTROL))),
    (33, "Application windows", Some((125, CONTROL))),
    (36, "Show Desktop", Some((103, 0))),
    (52, "Turn Dock hiding on/off", None),
    (57, "Move focus to status menus", None),
    (59, "Turn VoiceOver on or off", None),
    (
        60,
        "Select the previous input source",
        Some((SPACE, CONTROL)),
    ),
    (
        61,
        "Select next source in Input menu",
        Some((SPACE, CONTROL | OPTION)),
    ),
    (64, "Show Spotlight search", Some((SPACE, COMMAND))),
    (
        65,
        "Show Finder search window",
        Some((SPACE, OPTION | COMMAND)),
    ),
    (79, "Move left a space", Some((123, CONTROL))),
    (81, "Move right a space", Some((124, CONTROL))),
    (98, "Show Help menu", None),
    (160, "Show Launchpad", None),
    (162, "Show Accessibility controls", None),
    (163, "Show Notification Center", None),
    (164, "Turn Do Not Disturb on/off", None),
    (
        184,
        "Screenshot and recording options",
        Some((23, COMMAND | SHIFT)),
    ),
    (190, "Quick Note", None),
    (222, "Turn Stage Manager on/off", None),
];

/// The saved symbolic hot keys in `domain` (`com.apple.symbolichotkeys`), by
/// ID.
pub(crate) fn symbolic_hotkeys(domain: &Value) -> HashMap<u32, Entry> {
    let Some(keys) = domain
        .get("AppleSymbolicHotKeys")
        .and_then(Value::as_object)
    else {
        return HashMap::new();
    };
    keys.iter()
        .filter_map(|(id, entry)| {
            let id = id.parse().ok()?;
            let enabled = plist::integer(entry, "enabled").is_none_or(|on| on != 0);
            let chord = entry
                .get("value")
                .and_then(|value| value.get("parameters"))
                .and_then(Value::as_array)
                .filter(|parameters| parameters.len() >= 3)
                .and_then(|parameters| {
                    let code = parameters[1].as_i64().filter(|code| *code != NO_KEY)?;
                    let mods = parameters[2].as_i64()?;
                    Some((u16::try_from(code).ok()?, u64::try_from(mods).ok()?))
                });
            Some((id, Entry { enabled, chord }))
        })
        .collect()
}

/// The saved symbolic hot keys, read now.
#[allow(dead_code, reason = "read by the retired Screenshots shortcut check")]
pub(crate) fn read_symbolic_hotkeys() -> HashMap<u32, Entry> {
    symbolic_hotkeys(&plist::domain("com.apple.symbolichotkeys"))
}

/// What the 🌐 key does pressed alone (System Settings › Keyboard, "Press
/// 🌐 key to"; `AppleFnUsageType`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Globe {
    Nothing,
    ChangeInputSource,
    ShowEmoji,
    StartDictation,
    /// Not saved: macOS's default, which depends on the keyboard.
    #[default]
    Default,
}

/// macOS's settings that can take a shortcut's keys.
#[derive(Clone, Debug, Default)]
pub(crate) struct System {
    pub(crate) symbolic: HashMap<u32, Entry>,
    pub(crate) globe: Globe,
    pub(crate) dictation: bool,
    pub(crate) siri: bool,
}

impl System {
    pub(crate) fn read() -> Self {
        Self::from_domains(
            &plist::domain("com.apple.symbolichotkeys"),
            &plist::domain("com.apple.HIToolbox"),
            &plist::domain("com.apple.assistant.support"),
            &plist::domain("com.apple.Siri"),
        )
    }

    /// The settings the four domains hold: `com.apple.symbolichotkeys`,
    /// `com.apple.HIToolbox` (🌐 and Dictation), `com.apple.assistant.support`
    /// (Siri and Dictation on), and `com.apple.Siri` (its shortcut off).
    pub(crate) fn from_domains(
        symbolic: &Value,
        toolbox: &Value,
        assistant: &Value,
        siri: &Value,
    ) -> Self {
        let globe = match plist::integer(toolbox, "AppleFnUsageType") {
            Some(0) => Globe::Nothing,
            Some(1) => Globe::ChangeInputSource,
            Some(2) => Globe::ShowEmoji,
            Some(3) => Globe::StartDictation,
            _ => Globe::Default,
        };
        let on = |value: &Value, key: &str| plist::integer(value, key).is_some_and(|on| on != 0);
        Self {
            symbolic: symbolic_hotkeys(symbolic),
            globe,
            dictation: on(toolbox, "AppleDictationAutoEnable")
                || on(assistant, "Dictation Enabled"),
            // Older macOS saved Siri's keyboard shortcut as `HotkeyTag`, with
            // zero for Off (not verified on macOS 26, which saved none here).
            siri: on(assistant, "Assistant Enabled")
                && plist::integer(siri, "HotkeyTag") != Some(0),
        }
    }

    pub(crate) fn check(&self, binding: &HotkeyBinding, double_tap: bool) -> Vec<Conflict> {
        let mut found: Vec<Conflict> = self.symbolic_conflict(binding).into_iter().collect();
        let lone = Lone::of(binding);
        if lone == Some(Lone::Function) {
            let does = match self.globe {
                Globe::Nothing => None,
                Globe::ChangeInputSource => Some("changes the input source"),
                Globe::ShowEmoji => Some("shows Emoji & Symbols"),
                Globe::StartDictation => Some("starts Apple Dictation"),
                Globe::Default => Some("may do something of its own"),
            };
            if let Some(does) = does {
                found.push(Conflict {
                    source: Source::MacOs,
                    certainty: if self.globe == Globe::Default {
                        Certainty::Possible
                    } else {
                        Certainty::Taken
                    },
                    name: "🌐 key".into(),
                    description: format!(
                        "Pressing 🌐 alone also {does}. Set \"Press 🌐 key to\" to Do Nothing in Keyboard settings, or choose another shortcut."
                    ),
                    fix: Fix::KeyboardSettings,
                });
            }
        }
        let twice = match lone {
            Some(Lone::Command(_)) => Some("⌘"),
            Some(Lone::Control(_)) => Some("⌃"),
            Some(Lone::Function) => Some("🌐"),
            _ => None,
        };
        if double_tap
            && self.dictation
            && let Some(key) = twice
        {
            found.push(Conflict {
                source: Source::MacOs,
                certainty: Certainty::Possible,
                name: "Apple Dictation".into(),
                description: format!(
                    "Apple Dictation may start when you press {key} twice. Check its shortcut in Keyboard settings › Dictation."
                ),
                fix: Fix::KeyboardSettings,
            });
        }
        let siri_keys = match lone {
            Some(Lone::Command(_)) if double_tap => Some(format!("press {} twice", keys(binding))),
            _ => self.siri_hold(binding),
        };
        if self.siri
            && let Some(how) = siri_keys
        {
            found.push(Conflict {
                source: Source::MacOs,
                certainty: Certainty::Possible,
                name: "Siri".into(),
                description: format!(
                    "Siri may open when you {how}. Check its keyboard shortcut in Siri settings."
                ),
                fix: Fix::SiriSettings,
            });
        }
        found
    }

    /// Siri's hold-to-talk choices: ⌘ Space, ⌥ Space, and 🌐 Space.
    fn siri_hold(&self, binding: &HotkeyBinding) -> Option<String> {
        let runtime = binding.runtime();
        let m = binding.modifiers;
        (runtime.key_code == Some(SPACE)
            && m.count() == 1
            && (m.command.is_some() || m.option.is_some() || m.function))
            .then(|| format!("hold {}", keys(binding)))
    }

    /// The macOS keyboard shortcut that is on with `binding`'s chord.
    fn symbolic_conflict(&self, binding: &HotkeyBinding) -> Option<Conflict> {
        let runtime = binding.runtime();
        let code = runtime.key_code?;
        let wanted = flags(binding);
        let defaults = KNOWN
            .iter()
            .filter(|(id, ..)| !self.symbolic.contains_key(id))
            .filter_map(|&(id, _, chord)| Some((id, chord?)));
        let saved = self
            .symbolic
            .iter()
            .filter(|(_, entry)| entry.enabled)
            .filter_map(|(&id, entry)| Some((id, entry.chord?)));
        let mut matched: Vec<u32> = defaults
            .chain(saved)
            .filter(|&(_, (key, mods))| key == code && mods & MODIFIERS == wanted)
            .map(|(id, _)| id)
            .collect();
        matched.sort_unstable();
        let id = *matched.first()?;
        let name = KNOWN
            .iter()
            .find(|(known, ..)| *known == id)
            .map_or("a macOS keyboard shortcut", |(_, name, _)| name);
        Some(Conflict {
            source: Source::MacOs,
            certainty: Certainty::Taken,
            name: name.into(),
            description: format!(
                "macOS uses {} for \"{name}\", so Hex never sees it. Turn that off in Keyboard settings › Keyboard Shortcuts, or choose another shortcut.",
                keys(binding)
            ),
            fix: Fix::KeyboardSettings,
        })
    }
}

/// `binding`'s modifiers as symbolic hot keys save them (no sides).
fn flags(binding: &HotkeyBinding) -> u64 {
    let m = binding.modifiers;
    [
        (m.shift.is_some(), SHIFT),
        (m.control.is_some(), CONTROL),
        (m.option.is_some(), OPTION),
        (m.command.is_some(), COMMAND),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |all, (_, bit)| all | bit)
}
