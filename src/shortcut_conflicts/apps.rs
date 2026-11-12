//! Other apps known to take global keys: dictation apps, launchers,
//! remappers, and automation tools. Each counts only while it runs (or, for
//! Karabiner-Elements, while it is installed: its remapping runs in a
//! daemon). Where an app's setting is readable it is read: Raycast's hot key
//! (`raycastGlobalHotkey`) and Karabiner-Elements' rules
//! (`~/.config/karabiner/karabiner.json`); otherwise a running app is a
//! possible conflict for the keys it commonly takes.

use serde_json::Value;

use super::{Certainty, Conflict, Fix, Source, keys, plist};
use crate::app_settings::{HotkeyBinding, ModifierSide};

/// What kind of keys an app commonly takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Takes {
    /// Dictation apps: a held modifier (🌐, Right ⌘, Right ⌥) or ⌥ Space,
    /// the same keys Hex's shortcuts use. Any shortcut.
    Dictation,
    /// Launchers: ⌘ Space or ⌥ Space. Only a chord on Space.
    Launcher,
    /// Remappers and automation: anything the person set up. Any shortcut.
    Anything,
}

/// The apps checked: name, bundle IDs, and what they take.
const KNOWN: &[(&str, &[&str], Takes)] = &[
    (
        "Superwhisper",
        &["com.superduper.superwhisper"],
        Takes::Dictation,
    ),
    ("Wispr Flow", &["com.electron.wispr-flow"], Takes::Dictation),
    (
        "MacWhisper",
        &["com.goodsnooze.MacWhisper"],
        Takes::Dictation,
    ),
    (
        "Spokenly",
        &["app.spokenly", "com.spokenly.app"],
        Takes::Dictation,
    ),
    ("Raycast", &["com.raycast.macos"], Takes::Launcher),
    (
        "Alfred",
        &["com.runningwithcrayons.Alfred"],
        Takes::Launcher,
    ),
    (
        "BetterTouchTool",
        &["com.hegenberg.BetterTouchTool"],
        Takes::Anything,
    ),
    (
        "Hammerspoon",
        &["org.hammerspoon.Hammerspoon"],
        Takes::Anything,
    ),
    (
        "Keyboard Maestro",
        &["com.stairways.keyboardmaestro.engine"],
        Takes::Anything,
    ),
];

const KARABINER: &str = "Karabiner-Elements";
const KARABINER_BUNDLE: &str = "org.pqrs.Karabiner-Elements.Settings";
const SPACE: u16 = 49;

/// One known app that runs now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Running {
    pub(crate) name: String,
    pub(crate) bundle_id: String,
}

/// The known apps that can take keys now.
#[derive(Clone, Debug, Default)]
pub(crate) struct Apps {
    pub(crate) running: Vec<Running>,
    /// Raycast's saved hot key (`raycastGlobalHotkey`), such as
    /// `Command-49`.
    pub(crate) raycast_hotkey: Option<String>,
    /// Karabiner-Elements' configuration, while it is installed.
    pub(crate) karabiner: Option<Value>,
}

impl Apps {
    pub(crate) fn read() -> Self {
        let running = running_known_apps();
        let raycast_hotkey = running
            .iter()
            .any(|app| app.name == "Raycast")
            .then(|| {
                plist::domain("com.raycast.macos")
                    .get("raycastGlobalHotkey")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .flatten();
        let karabiner = std::path::Path::new("/Applications/Karabiner-Elements.app")
            .exists()
            .then(|| {
                let path = crate::app_paths::home_dir()?.join(".config/karabiner/karabiner.json");
                serde_json::from_slice(&std::fs::read(path).ok()?).ok()
            })
            .flatten();
        Self {
            running,
            raycast_hotkey,
            karabiner,
        }
    }

    pub(crate) fn check(&self, binding: &HotkeyBinding) -> Vec<Conflict> {
        let mut found = Vec::new();
        let on_space = binding.runtime().key_code == Some(SPACE);
        for app in &self.running {
            let Some(&(_, _, takes)) = KNOWN.iter().find(|(name, ..)| *name == app.name) else {
                continue;
            };
            let fix = Fix::OpenApp {
                bundle_id: app.bundle_id.clone(),
            };
            if app.name == "Raycast"
                && let Some(saved) = self.raycast_hotkey.as_deref().and_then(raycast_hotkey)
            {
                if saved == (binding.runtime().key_code, launcher_flags(binding)) {
                    found.push(taken(app, binding, fix));
                }
                continue;
            }
            let relevant = match takes {
                Takes::Dictation | Takes::Anything => true,
                Takes::Launcher => on_space,
            };
            if relevant {
                found.push(Conflict {
                    source: Source::App(app.name.clone()),
                    certainty: Certainty::Possible,
                    name: app.name.clone(),
                    description: format!(
                        "{} is running and may use {} too. If the shortcut doesn't work, change it in {}, or choose another shortcut.",
                        app.name,
                        keys(binding),
                        app.name
                    ),
                    fix,
                });
            }
        }
        if let Some(config) = &self.karabiner {
            found.extend(karabiner_conflict(config, binding));
        }
        found
    }
}

fn taken(app: &Running, binding: &HotkeyBinding, fix: Fix) -> Conflict {
    Conflict {
        source: Source::App(app.name.clone()),
        certainty: Certainty::Taken,
        name: app.name.clone(),
        description: format!(
            "{} uses {} as its own shortcut. Change it in {}, or choose another shortcut.",
            app.name,
            keys(binding),
            app.name
        ),
        fix,
    }
}

/// The modifiers Raycast's hot key names, as bits: ⌘ 1, ⌥ 2, ⌃ 4, ⇧ 8.
fn launcher_flags(binding: &HotkeyBinding) -> u8 {
    let m = binding.modifiers;
    u8::from(m.command.is_some())
        | u8::from(m.option.is_some()) << 1
        | u8::from(m.control.is_some()) << 2
        | u8::from(m.shift.is_some()) << 3
}

/// Raycast's saved hot key, such as `Command-49` or `Option-Shift-49`: its
/// key code and modifier bits ([`launcher_flags`]). `None` when it can't be
/// read.
pub(super) fn raycast_hotkey(saved: &str) -> Option<(Option<u16>, u8)> {
    let mut parts: Vec<&str> = saved.split('-').collect();
    let code = parts.pop()?.trim().parse::<u16>().ok()?;
    let mut flags = 0;
    for part in parts {
        flags |= match part.trim().to_ascii_lowercase().as_str() {
            "command" | "cmd" => 1,
            "option" | "alt" => 2,
            "control" | "ctrl" => 4,
            "shift" => 8,
            _ => return None,
        };
    }
    Some((Some(code), flags))
}

/// Whether Karabiner-Elements' selected profile changes one of `binding`'s
/// keys: a simple modification makes it another key (Hex never sees it), and
/// a complex rule starting from it may.
fn karabiner_conflict(config: &Value, binding: &HotkeyBinding) -> Option<Conflict> {
    let names = karabiner_names(binding);
    if names.is_empty() {
        return None;
    }
    let profiles = config.get("profiles")?.as_array()?;
    let profile = profiles
        .iter()
        .find(|profile| profile.get("selected").and_then(Value::as_bool) == Some(true))
        .or(profiles.first())?;
    let from_key = |entry: &Value| {
        entry
            .get("from")
            .and_then(|from| from.get("key_code"))
            .and_then(Value::as_str)
            .is_some_and(|key| names.contains(&key))
    };
    let devices = profile
        .get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let simple = std::iter::once(profile)
        .chain(devices)
        .filter_map(|holder| holder.get("simple_modifications")?.as_array())
        .flatten()
        .any(from_key);
    let complex = profile
        .get("complex_modifications")
        .and_then(|complex| complex.get("rules"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|rule| rule.get("manipulators")?.as_array())
        .flatten()
        .any(from_key);
    let certainty = if simple {
        Certainty::Taken
    } else if complex {
        Certainty::Possible
    } else {
        return None;
    };
    Some(Conflict {
        source: Source::App(KARABINER.into()),
        certainty,
        name: KARABINER.into(),
        description: if simple {
            format!(
                "{KARABINER} turns a key of {} into another key, so Hex never sees it. Remove that change in {KARABINER}, or choose another shortcut.",
                keys(binding)
            )
        } else {
            format!(
                "A {KARABINER} rule changes a key of {}. If the shortcut doesn't work, check that rule, or choose another shortcut.",
                keys(binding)
            )
        },
        fix: Fix::OpenApp {
            bundle_id: KARABINER_BUNDLE.into(),
        },
    })
}

/// Karabiner-Elements' names for `binding`'s keys.
fn karabiner_names(binding: &HotkeyBinding) -> Vec<&'static str> {
    let m = binding.modifiers;
    let mut names = Vec::new();
    let mut sided = |side: Option<ModifierSide>, left: &'static str, right: &'static str| match side
    {
        None => {}
        Some(ModifierSide::Left) => names.push(left),
        Some(ModifierSide::Right) => names.push(right),
        Some(ModifierSide::Either | ModifierSide::Both) => names.extend([left, right]),
    };
    sided(m.command, "left_command", "right_command");
    sided(m.option, "left_option", "right_option");
    sided(m.control, "left_control", "right_control");
    sided(m.shift, "left_shift", "right_shift");
    if m.function {
        names.push("fn");
    }
    if let Some(name) = binding.runtime().key_code.and_then(karabiner_key) {
        names.push(name);
    }
    names
}

/// Karabiner-Elements' name for a macOS virtual key code (its physical key).
fn karabiner_key(code: u16) -> Option<&'static str> {
    const KEYS: &[(u16, &str)] = &[
        (0, "a"),
        (1, "s"),
        (2, "d"),
        (3, "f"),
        (4, "h"),
        (5, "g"),
        (6, "z"),
        (7, "x"),
        (8, "c"),
        (9, "v"),
        (10, "non_us_backslash"),
        (11, "b"),
        (12, "q"),
        (13, "w"),
        (14, "e"),
        (15, "r"),
        (16, "y"),
        (17, "t"),
        (31, "o"),
        (32, "u"),
        (34, "i"),
        (35, "p"),
        (36, "return_or_enter"),
        (37, "l"),
        (38, "j"),
        (40, "k"),
        (45, "n"),
        (46, "m"),
        (48, "tab"),
        (49, "spacebar"),
        (50, "grave_accent_and_tilde"),
        (53, "escape"),
        (57, "caps_lock"),
        (96, "f5"),
        (97, "f6"),
        (98, "f7"),
        (99, "f3"),
        (100, "f8"),
        (101, "f9"),
        (103, "f11"),
        (105, "f13"),
        (107, "f14"),
        (109, "f10"),
        (111, "f12"),
        (113, "f15"),
        (118, "f4"),
        (120, "f2"),
        (122, "f1"),
    ];
    KEYS.iter()
        .find(|(key, _)| *key == code)
        .map(|(_, name)| *name)
}

/// The known apps running now, by bundle ID or by name.
fn running_known_apps() -> Vec<Running> {
    use objc2_app_kit::NSWorkspace;
    let running = NSWorkspace::sharedWorkspace().runningApplications();
    let mut found: Vec<Running> = Vec::new();
    for index in 0..running.count() {
        let app = running.objectAtIndex(index);
        let bundle_id = app
            .bundleIdentifier()
            .map(|id| id.to_string())
            .unwrap_or_default();
        let name = app
            .localizedName()
            .map(|name| name.to_string())
            .unwrap_or_default();
        let known = KNOWN.iter().find(|(known, ids, _)| {
            ids.contains(&bundle_id.as_str()) || known.eq_ignore_ascii_case(&name)
        });
        if let Some((known, ..)) = known
            && !found.iter().any(|app| app.name == *known)
        {
            found.push(Running {
                name: (*known).into(),
                bundle_id,
            });
        }
    }
    found
}
