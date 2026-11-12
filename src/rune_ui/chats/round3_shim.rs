//! TEMPORARY SHIM: remove at integration.
//!
//! Stand-ins, with the brain's and the voice owner's names, for what round
//! three's Settings and onboarding build on and that isn't in this
//! worktree yet:
//!
//! - `assistant::memory::{read, write}` → [`memory`]
//! - `assistant::schedules::{list, add, remove, set_enabled, next_runs}`
//!   and its `Schedule` → [`schedules`] (kept in memory only)
//! - `speech::better_voice_available(lang)` and
//!   `speech::open_voice_settings()`, and a voice's quality → [`voices`]
//!
//! At integration: delete this file and `mod round3_shim;`, and read
//! `round3_shim::memory::` as `crate::assistant::memory::`,
//! `round3_shim::schedules::` as `crate::assistant::schedules::`, and
//! `round3_shim::voices::` as `crate::assistant::speech::`.

pub(in crate::rune_ui) mod memory {
    use std::sync::Mutex;

    use color_eyre::Result;

    static MEMORY: Mutex<String> = Mutex::new(String::new());

    /// What Hex knows about the person, as Markdown lines.
    pub(in crate::rune_ui) fn read() -> String {
        MEMORY.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    pub(in crate::rune_ui) fn write(text: &str) -> Result<()> {
        *MEMORY.lock().unwrap_or_else(|error| error.into_inner()) = text.to_owned();
        Ok(())
    }
}

pub(in crate::rune_ui) mod schedules {
    use std::sync::Mutex;

    use color_eyre::{Result, eyre::eyre};

    /// How often a schedule repeats.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(in crate::rune_ui) enum Repeat {
        Once,
        Daily,
        Weekdays,
        Weekly,
    }

    /// One scheduled request.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(in crate::rune_ui) struct Schedule {
        pub(in crate::rune_ui) id: String,
        /// What to ask, as the person said it.
        pub(in crate::rune_ui) text: String,
        /// Minutes after midnight, local time.
        pub(in crate::rune_ui) minute_of_day: u32,
        pub(in crate::rune_ui) repeat: Repeat,
        pub(in crate::rune_ui) enabled: bool,
        /// The next run, Unix ms; `None` while off or done.
        pub(in crate::rune_ui) next_ms: Option<u64>,
    }

    static SCHEDULES: Mutex<Vec<Schedule>> = Mutex::new(Vec::new());

    fn all() -> std::sync::MutexGuard<'static, Vec<Schedule>> {
        SCHEDULES.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(in crate::rune_ui) fn list() -> Vec<Schedule> {
        all().clone()
    }

    pub(in crate::rune_ui) fn add(text: &str, minute_of_day: u32, repeat: Repeat) -> Result<String> {
        if text.trim().is_empty() {
            return Err(eyre!("Say what it should do"));
        }
        let mut all = all();
        let id = format!("schedule-{}", all.len() + 1);
        all.push(Schedule {
            id: id.clone(),
            text: text.trim().into(),
            minute_of_day,
            repeat,
            enabled: true,
            next_ms: None,
        });
        Ok(id)
    }

    pub(in crate::rune_ui) fn remove(id: &str) {
        all().retain(|schedule| schedule.id != id);
    }

    pub(in crate::rune_ui) fn set_enabled(id: &str, enabled: bool) {
        if let Some(schedule) = all().iter_mut().find(|schedule| schedule.id == id) {
            schedule.enabled = enabled;
        }
    }

    /// Runs `id` now, outside its schedule.
    pub(in crate::rune_ui) fn run_now(id: &str) -> Option<String> {
        all()
            .iter()
            .find(|schedule| schedule.id == id)
            .map(|schedule| schedule.text.clone())
    }
}

pub(in crate::rune_ui) mod voices {
    /// How good an installed voice sounds.
    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    pub(in crate::rune_ui) enum Quality {
        Default,
        Enhanced,
        Premium,
    }

    /// A voice's quality, from its name ("Ava (Premium)").
    pub(in crate::rune_ui) fn quality(name: &str) -> Quality {
        if name.contains("Premium") {
            Quality::Premium
        } else if name.contains("Enhanced") {
            Quality::Enhanced
        } else {
            Quality::Default
        }
    }

    /// A hint when a better voice than the installed ones exists for
    /// `language` (none is Enhanced or Premium).
    pub(in crate::rune_ui) fn better_voice_available(installed: &[String]) -> Option<&'static str> {
        (!installed
            .iter()
            .any(|voice| quality(voice) != Quality::Default))
        .then_some("Premium voices sound far more natural")
    }

    /// System Settings at Spoken Content, where voices are downloaded.
    pub(in crate::rune_ui) fn open_voice_settings() {
        let spawned = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.universalaccess?SpokenContent")
            .spawn();
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not open the voice settings");
        }
    }
}
