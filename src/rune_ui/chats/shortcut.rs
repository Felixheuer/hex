//! The onboarding's view of the trigger's shortcut checks: what else takes a
//! shortcut (`shortcut_conflicts`) and the live "press it now" test
//! (`recognition::shortcut_probe`), both macOS only. Elsewhere there is no
//! Assistant, and these report that they can't run.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use crate::app_settings::{AppSettings, HotkeySlot};

/// Something that takes, or may take, a shortcut's keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) struct Conflict {
    /// "Siri", "Spotlight", "Raycast".
    pub(in crate::rune_ui) name: String,
    /// One sentence for a person.
    pub(in crate::rune_ui) description: String,
    /// A saved setting says it's taken; otherwise it only may be.
    pub(in crate::rune_ui) taken: bool,
    pub(in crate::rune_ui) fix: ConflictFix,
}

/// Where a conflict is resolved (macOS reads them).
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(in crate::rune_ui) enum ConflictFix {
    KeyboardSettings,
    SiriSettings,
    /// The other app, by its bundle id.
    OpenApp(String),
}

impl ConflictFix {
    /// Opens where the fix happens (returns at once).
    pub(in crate::rune_ui) fn open(&self) {
        #[cfg(target_os = "macos")]
        {
            use crate::shortcut_conflicts::Fix;
            match self {
                Self::KeyboardSettings => Fix::KeyboardSettings,
                Self::SiriSettings => Fix::SiriSettings,
                Self::OpenApp(bundle_id) => Fix::OpenApp {
                    bundle_id: bundle_id.clone(),
                },
            }
            .open();
        }
    }
}

/// What everything on this Mac does with `slot`'s saved shortcut. Reads
/// preferences and the running apps; off the window's thread.
pub(in crate::rune_ui) fn conflicts(slot: HotkeySlot, settings: &AppSettings) -> Vec<Conflict> {
    #[cfg(target_os = "macos")]
    {
        use crate::shortcut_conflicts::{Certainty, Fix};
        crate::shortcut_conflicts::conflicts(slot, settings)
            .into_iter()
            .map(|conflict| Conflict {
                name: conflict.name,
                description: conflict.description,
                taken: conflict.certainty == Certainty::Taken,
                fix: match conflict.fix {
                    Fix::KeyboardSettings => ConflictFix::KeyboardSettings,
                    Fix::SiriSettings => ConflictFix::SiriSettings,
                    Fix::OpenApp { bundle_id } => ConflictFix::OpenApp(bundle_id),
                },
            })
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (slot, settings);
        Vec::new()
    }
}

/// What a press test found (macOS runs them).
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(in crate::rune_ui) enum Probe {
    /// Its press and release reached Hex with exactly its keys.
    Works(Duration),
    /// The press reached Hex, but the release never did.
    NoRelease,
    /// The keyboard sent the key, but another app or macOS took it first.
    Swallowed,
    /// Its keys came with other modifiers or sides: what was held.
    WrongKeys(String),
    /// Nothing reached Hex in time.
    NothingReceived,
    /// The test couldn't run; why, for a person.
    Unavailable(String),
}

/// Starts testing `slot`'s saved shortcut; the answer comes once it's
/// pressed and released, or after `wait`. One test runs at a time; while it
/// does, the shortcut's keys go to it and start nothing.
pub(in crate::rune_ui) fn start_probe(slot: HotkeySlot, wait: Duration) -> Receiver<Probe> {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    #[cfg(target_os = "macos")]
    {
        use crate::recognition::shortcut_probe::{self, ProbeResult};
        let result = shortcut_probe::start(slot, wait);
        let forwarded = std::thread::Builder::new()
            .name("hex-assistant-probe".into())
            .spawn(move || {
                let Ok(result) = result.recv() else {
                    return;
                };
                let _ = reply.send(match result {
                    ProbeResult::Works { held } => Probe::Works(held),
                    ProbeResult::NoRelease => Probe::NoRelease,
                    ProbeResult::Swallowed => Probe::Swallowed,
                    ProbeResult::WrongKeys { saw } => Probe::WrongKeys(saw),
                    ProbeResult::NothingReceived => Probe::NothingReceived,
                    ProbeResult::Unavailable(why) => Probe::Unavailable(why),
                });
            });
        if let Err(error) = forwarded {
            tracing::warn!(%error, "could not follow a shortcut test");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (slot, wait);
        let _ = reply.send(Probe::Unavailable(String::new()));
    }
    answer
}

/// Ends a running test without an answer.
pub(in crate::rune_ui) fn cancel_probe() {
    #[cfg(target_os = "macos")]
    crate::recognition::shortcut_probe::cancel();
}
