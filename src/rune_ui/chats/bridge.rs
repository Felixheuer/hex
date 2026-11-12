//! What Settings, the onboarding, and Chats' Scheduled section read and
//! change, in the shapes those views draw, backed by the assistant's own
//! modules: `assistant::memory`, `assistant::schedules`, and
//! `assistant::speech`.

pub(in crate::rune_ui) mod memory {
    pub(in crate::rune_ui) use crate::assistant::memory::{read, write};
}

pub(in crate::rune_ui) mod schedules {
    use color_eyre::{Result, eyre::eyre};

    use crate::assistant::Lane;
    use crate::assistant::schedules as store;

    /// How often a schedule repeats, as the New schedule sheet offers it.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(in crate::rune_ui) enum Repeat {
        Once,
        Daily,
        Weekdays,
        Weekly,
        /// Made by voice or chat with a rule the sheet doesn't offer
        /// ("every first Monday"); shown, never offered.
        Custom,
    }

    /// One scheduled request.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(in crate::rune_ui) struct Schedule {
        pub(in crate::rune_ui) id: String,
        /// What to ask, as the person said it.
        pub(in crate::rune_ui) text: String,
        /// Minutes after midnight, local time (0 for a `Custom` rule
        /// without one fixed time).
        pub(in crate::rune_ui) minute_of_day: u32,
        pub(in crate::rune_ui) repeat: Repeat,
        pub(in crate::rune_ui) enabled: bool,
        /// The next run, Unix ms; `None` while off or done.
        pub(in crate::rune_ui) next_ms: Option<u64>,
    }

    /// The sheet's shape of a stored rule: its time and how it repeats.
    pub(in crate::rune_ui) fn shape(when: &str, once: bool) -> (u32, Repeat) {
        let fields: Vec<&str> = when.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return (0, Repeat::Custom);
        };
        let (Ok(minute), Ok(hour)) = (minute.parse::<u32>(), hour.parse::<u32>()) else {
            return (0, Repeat::Custom);
        };
        let time = hour * 60 + minute;
        let number = |field: &str| field.parse::<u32>().is_ok();
        let repeat = match (day, month, weekday) {
            _ if once => Repeat::Once,
            ("*", "*", "*") => Repeat::Daily,
            ("*", "*", "1-5") => Repeat::Weekdays,
            ("*", "*", weekday) if number(weekday) => Repeat::Weekly,
            _ => Repeat::Custom,
        };
        (time, repeat)
    }

    /// The cron line for the sheet's choice, `today` being local
    /// (day, month, weekday with 0 as Sunday) and `passed` whether the time
    /// already went by today.
    pub(in crate::rune_ui) fn rule(
        minute_of_day: u32,
        repeat: Repeat,
        today: (u32, u32, u32),
        tomorrow: (u32, u32),
        passed: bool,
    ) -> Result<String> {
        let (minute, hour) = (minute_of_day % 60, minute_of_day / 60);
        if hour > 23 {
            return Err(eyre!("Pick a time of day"));
        }
        Ok(match repeat {
            Repeat::Daily => format!("{minute} {hour} * * *"),
            Repeat::Weekdays => format!("{minute} {hour} * * 1-5"),
            Repeat::Weekly => format!("{minute} {hour} * * {}", today.2),
            Repeat::Once => {
                let (day, month) = if passed { tomorrow } else { (today.0, today.1) };
                format!("{minute} {hour} {day} {month} *")
            }
            Repeat::Custom => return Err(eyre!("Pick how often it repeats")),
        })
    }

    fn from_store(schedule: &store::Schedule) -> Schedule {
        let (minute_of_day, repeat) = shape(&schedule.when, schedule.once);
        Schedule {
            id: schedule.id.clone(),
            text: schedule.task.clone(),
            minute_of_day,
            repeat,
            enabled: schedule.enabled,
            next_ms: schedule.enabled.then(|| schedule.next_due()).flatten(),
        }
    }

    pub(in crate::rune_ui) fn list() -> Vec<Schedule> {
        store::list().iter().map(from_store).collect()
    }

    /// Adds a Background schedule and returns its id.
    pub(in crate::rune_ui) fn add(
        text: &str,
        minute_of_day: u32,
        repeat: Repeat,
    ) -> Result<String> {
        if text.trim().is_empty() {
            return Err(eyre!("Say what it should do"));
        }
        let now = crate::history::now_ms();
        let local = store::local(now);
        let next_day = store::local(now + 24 * 60 * 60 * 1000);
        let passed = local.hour * 60 + local.minute >= minute_of_day;
        let when = rule(
            minute_of_day,
            repeat,
            (local.day, local.month, local.weekday),
            (next_day.day, next_day.month),
            passed,
        )?;
        let added = if repeat == Repeat::Once {
            store::add_once(&when, text, Lane::Background)?
        } else {
            store::add(&when, text, Lane::Background)?
        };
        Ok(added.id)
    }

    pub(in crate::rune_ui) fn remove(id: &str) {
        store::remove(id);
    }

    pub(in crate::rune_ui) fn set_enabled(id: &str, enabled: bool) {
        store::set_enabled(id, enabled);
    }

    /// The text to run now, outside its schedule (Chats submits it).
    pub(in crate::rune_ui) fn run_now(id: &str) -> Option<String> {
        store::list()
            .into_iter()
            .find(|schedule| schedule.id == id)
            .map(|schedule| schedule.task)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_sheets_choices_round_trip_through_cron() {
            let today = (12, 11, 3);
            for repeat in [Repeat::Daily, Repeat::Weekdays, Repeat::Weekly] {
                let when = rule(7 * 60 + 30, repeat, today, (13, 11), false).unwrap();
                assert_eq!(shape(&when, false), (7 * 60 + 30, repeat), "{when}");
            }
            let once = rule(8 * 60, Repeat::Once, today, (13, 11), true).unwrap();
            assert_eq!(once, "0 8 13 11 *");
            assert_eq!(shape(&once, true), (8 * 60, Repeat::Once));
            assert_eq!(shape("FREQ=MONTHLY;BYDAY=1MO", false).1, Repeat::Custom);
            assert_eq!(shape("0 9 1 * *", false), (9 * 60, Repeat::Custom));
            assert!(rule(25 * 60, Repeat::Daily, today, (13, 11), false).is_err());
        }
    }
}

pub(in crate::rune_ui) mod voices {
    use crate::assistant::speech;

    pub(in crate::rune_ui) use crate::assistant::speech::{Quality, open_voice_settings};

    /// An installed voice's quality, by the name Settings shows.
    pub(in crate::rune_ui) fn quality(name: &str) -> Quality {
        speech::voices_with_languages()
            .into_iter()
            .find(|voice| voice.name == name)
            .map_or(Quality::Default, |voice| voice.quality)
    }

    /// A hint when a much better voice than the installed ones can be
    /// downloaded for the system's language.
    pub(in crate::rune_ui) fn better_voice_available(
        _installed: &[String],
    ) -> Option<&'static str> {
        let language = speech::language_of(&system_sample())?;
        speech::better_voice_available(language).map(|_| "Premium voices sound far more natural")
    }

    /// A short phrase in the system's language, for picking its language.
    fn system_sample() -> String {
        let preferred = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default();
        if preferred.starts_with("de") {
            "Das ist ein kurzer Satz auf Deutsch.".into()
        } else if preferred.starts_with("fr") {
            "C'est une courte phrase en français.".into()
        } else if preferred.starts_with("es") {
            "Esta es una frase corta en español.".into()
        } else {
            "This is a short sentence in English.".into()
        }
    }
}
