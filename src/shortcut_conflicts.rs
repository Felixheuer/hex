//! Whether an Agent-Desk shortcut can work on this Mac, or something takes it first
//! (macOS): macOS's own keyboard shortcuts (`com.apple.symbolichotkeys`),
//! the 🌐 key's own action, Apple Dictation's and Siri's press-twice and
//! hold shortcuts, and other apps known to take global keys (dictation apps,
//! launchers, remappers, automation tools).
//!
//! Each check is either certain (`Certainty::Taken`: a saved setting names
//! the same keys) or a warning (`Certainty::Possible`: the other side is on or
//! running and commonly uses such keys, but what it uses can't be read).
//! `recognition::shortcut_probe` answers the rest by asking for a press.
//!
//! ```ts
//! conflicts(slot, settings)
//! ├── Environment::read()     // the saved preferences and the running apps, now
//! │   ├── system::read        // symbolic hot keys, 🌐, Apple Dictation, Siri
//! │   └── apps::read          // running known apps, Raycast's hot key, Karabiner's rules
//! └── check(binding, double_tap, environment)   // pure; fixture-tested
//! ```

use crate::app_settings::{AppSettings, HotkeyBinding, HotkeySlot, ModifierSide};

mod apps;
mod plist;
mod system;

pub(crate) use system::{Entry, read_symbolic_hotkeys};

/// Who takes the keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Source {
    /// One of macOS's keyboard shortcuts, the 🌐 key, Apple Dictation, or
    /// Siri.
    MacOs,
    /// Another app, by name.
    App(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Certainty {
    /// A saved setting says the keys are taken: Agent-Desk never sees them, or they
    /// do something else too.
    Taken,
    /// The other side is on or running and commonly uses such keys; pressing
    /// the shortcut (`shortcut_probe`) tells for sure.
    Possible,
}

/// What resolves it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Fix {
    /// System Settings › Keyboard, where Keyboard Shortcuts, Dictation, and
    /// the 🌐 key's action live.
    KeyboardSettings,
    /// System Settings › Siri, where its keyboard shortcut lives.
    SiriSettings,
    /// The other app, where its shortcut is changed.
    OpenApp { bundle_id: String },
}

impl Fix {
    /// Opens where the fix happens. Choosing another shortcut is always a
    /// fix too, and the caller's.
    pub(crate) fn open(&self) {
        let mut command = std::process::Command::new("/usr/bin/open");
        match self {
            Self::KeyboardSettings => {
                command.arg("x-apple.systempreferences:com.apple.Keyboard-Settings.extension")
            }
            Self::SiriSettings => {
                command.arg("x-apple.systempreferences:com.apple.Siri-Settings.extension")
            }
            Self::OpenApp { bundle_id } => command.args(["-b", bundle_id]),
        };
        if let Err(error) = command.spawn() {
            tracing::warn!(%error, ?self, "could not open where a shortcut conflict is fixed");
        }
    }
}

/// One thing that takes, or may take, a shortcut's keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Conflict {
    pub(crate) source: Source,
    pub(crate) certainty: Certainty,
    /// What takes it, as a person reads it: "Spotlight", "Siri", "Raycast".
    pub(crate) name: String,
    /// One English sentence for a person.
    pub(crate) description: String,
    pub(crate) fix: Fix,
}

/// Everything `slot`'s saved shortcut may collide with on this Mac now. Reads
/// preferences and the running apps (a few milliseconds); `cached` is for
/// views drawn every frame. A slot with no shortcut has none.
pub(crate) fn conflicts(slot: HotkeySlot, settings: &AppSettings) -> Vec<Conflict> {
    let Some((binding, double_tap)) = shortcut(slot, settings) else {
        return Vec::new();
    };
    check(&binding, double_tap, &Environment::read())
}

/// [`conflicts`], read again at most every 3 s per slot and binding.
pub(crate) fn cached(slot: HotkeySlot, settings: &AppSettings) -> Vec<Conflict> {
    use std::time::{Duration, Instant};
    type Last = Vec<(
        HotkeySlot,
        Option<(HotkeyBinding, bool)>,
        Instant,
        Vec<Conflict>,
    )>;
    static LAST: std::sync::Mutex<Last> = std::sync::Mutex::new(Vec::new());
    let wanted = shortcut(slot, settings);
    let mut last = LAST.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((.., found)) = last.iter().find(|(s, binding, read, _)| {
        *s == slot && *binding == wanted && read.elapsed() < Duration::from_secs(3)
    }) {
        return found.clone();
    }
    let found = conflicts(slot, settings);
    last.retain(|(s, ..)| *s != slot);
    last.push((slot, wanted, Instant::now(), found.clone()));
    found
}

/// `slot`'s saved shortcut, and whether it is used with a double tap (the
/// Action shortcut's task list; dictation's double-tap lock).
fn shortcut(slot: HotkeySlot, settings: &AppSettings) -> Option<(HotkeyBinding, bool)> {
    let binding = match slot {
        HotkeySlot::Dictation => Some(&settings.dictation_hotkey),
        HotkeySlot::Action => settings.assistant.action_hotkey.as_ref(),
        HotkeySlot::StopAll => settings.assistant.stop_all_hotkey.as_ref(),
        HotkeySlot::SecondDictation => settings.second_dictation_hotkey.as_ref(),
        HotkeySlot::RawDictation => settings.raw_dictation_hotkey.as_ref(),
        HotkeySlot::PasteLast => settings.paste_last_hotkey.as_ref(),
        _ => None,
    }?;
    let double_tap = match slot {
        HotkeySlot::Action => true,
        HotkeySlot::StopAll => false,
        _ => settings.double_tap_lock,
    };
    Some((binding.clone(), double_tap))
}

/// What on this Mac can take keys, as read now.
#[derive(Clone, Debug, Default)]
pub(crate) struct Environment {
    pub(crate) system: system::System,
    pub(crate) apps: apps::Apps,
}

impl Environment {
    pub(crate) fn read() -> Self {
        Self {
            system: system::System::read(),
            apps: apps::Apps::read(),
        }
    }
}

/// Everything in `environment` that takes, or may take, `binding`.
/// `double_tap`: the shortcut is also pressed twice.
pub(crate) fn check(
    binding: &HotkeyBinding,
    double_tap: bool,
    environment: &Environment,
) -> Vec<Conflict> {
    let mut found = environment.system.check(binding, double_tap);
    found.extend(environment.apps.check(binding));
    found
}

/// A lone modifier key, as the press-twice shortcuts name them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Lone {
    Command(ModifierSide),
    Control(ModifierSide),
    Option(ModifierSide),
    Shift(ModifierSide),
    Function,
}

impl Lone {
    pub(crate) fn of(binding: &HotkeyBinding) -> Option<Self> {
        if binding.key.is_some() || binding.modifiers.count() != 1 {
            return None;
        }
        let m = binding.modifiers;
        if m.function {
            return Some(Self::Function);
        }
        m.command
            .map(Self::Command)
            .or(m.control.map(Self::Control))
            .or(m.option.map(Self::Option))
            .or(m.shift.map(Self::Shift))
    }
}

/// The shortcut's keys as a person reads them: "⌥ Space", "Right ⌘".
pub(crate) fn keys(binding: &HotkeyBinding) -> String {
    binding.keycaps().join(" ")
}

#[cfg(test)]
mod tests;
