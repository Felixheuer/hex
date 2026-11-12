//! Jobs that run on a schedule ("every morning at 8, check …"): kept in
//! `<support>/assistant/schedules.json`, readable only by this user, and
//! started by the task runner's scheduler (`service`) as Background jobs.
//! A schedule says when as a cron line (`0 8 * * 1-5`) or an iCalendar
//! RRULE (`FREQ=WEEKLY;BYDAY=MO;BYHOUR=9`), in this computer's local time.
//!
//! Runs missed while the Mac slept or Hex was closed run once when Hex next
//! looks, never once per missed time. Tasks add, list, and remove schedules
//! with `hex_schedule`, `hex_schedules`, and `hex_unschedule` (`tools`); the
//! UI uses [`list`], [`add`], [`remove`], [`set_enabled`], [`next_runs`].
//!
//! OWNER: brain.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use color_eyre::{Result, eyre::eyre};
use serde::{Deserialize, Serialize};

use crate::assistant::store;
use crate::assistant::types::Lane;

/// The furthest ahead a schedule is looked for its next time.
const HORIZON_MINUTES: u64 = 400 * 24 * 60;

/// One scheduled job.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Schedule {
    pub id: String,
    /// A cron line or an RRULE, local time.
    pub when: String,
    /// What to do, as the person would say it.
    pub task: String,
    /// Scheduled runs are Background jobs; `Here` is kept as asked but runs
    /// in the background too (there's no window in front to act on).
    pub lane: Lane,
    pub enabled: bool,
    pub created_ms: u64,
    /// When it last ran (Unix ms).
    #[serde(default)]
    pub last_run_ms: Option<u64>,
    /// Runs one time, then removes itself (a dated cron line would
    /// otherwise come back on the same date next year).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once: bool,
}

impl Schedule {
    /// When it runs next after it last ran (or was made); `None` when its
    /// rule never matches again.
    pub fn next_due(&self) -> Option<u64> {
        let rule = Rule::parse(&self.when).ok()?;
        rule.next_after(self.last_run_ms.unwrap_or(self.created_ms))
    }
}

fn dir() -> Result<PathBuf> {
    store::assistant_dir()
}

/// Every schedule, oldest first.
pub fn list() -> Vec<Schedule> {
    dir().map(|dir| load(&dir)).unwrap_or_default()
}

/// Adds a schedule for `task` at `when`.
pub fn add(when: &str, task: &str, lane: Lane) -> Result<Schedule> {
    add_with(when, task, lane, false)
}

/// Adds a schedule that runs at `when`'s next time only, then removes itself.
pub fn add_once(when: &str, task: &str, lane: Lane) -> Result<Schedule> {
    add_with(when, task, lane, true)
}

fn add_with(when: &str, task: &str, lane: Lane, once: bool) -> Result<Schedule> {
    let when = when.trim();
    let rule = Rule::parse(when)?;
    let now = store::now_ms();
    if rule.next_after(now).is_none() {
        return Err(eyre!("“{when}” never runs"));
    }
    let task = task.trim();
    if task.is_empty() {
        return Err(eyre!("a schedule needs a task"));
    }
    let schedule = Schedule {
        id: store::new_id()[..8].to_owned(),
        when: when.to_owned(),
        task: task.to_owned(),
        lane,
        enabled: true,
        created_ms: now,
        last_run_ms: None,
        once,
    };
    let added = schedule.clone();
    change(&dir()?, move |list| list.push(added))?;
    Ok(schedule)
}

/// Removes schedule `id`; whether there was one.
pub fn remove(id: &str) -> bool {
    let mut found = false;
    let _ = dir().and_then(|dir| {
        change(&dir, |list| {
            let before = list.len();
            list.retain(|schedule| schedule.id != id);
            found = list.len() != before;
        })
    });
    found
}

/// Turns schedule `id` on or off; whether there was one.
pub fn set_enabled(id: &str, enabled: bool) -> bool {
    let mut found = false;
    let _ = dir().and_then(|dir| {
        change(&dir, |list| {
            if let Some(schedule) = list.iter_mut().find(|schedule| schedule.id == id) {
                schedule.enabled = enabled;
                found = true;
            }
        })
    });
    found
}

/// The next `count` times (Unix ms) `when` runs, from now.
pub fn next_runs(when: &str, count: usize) -> Result<Vec<u64>> {
    let rule = Rule::parse(when)?;
    let mut runs = Vec::new();
    let mut after = store::now_ms();
    while runs.len() < count {
        let Some(next) = rule.next_after(after) else {
            break;
        };
        runs.push(next);
        after = next;
    }
    Ok(runs)
}

/// The schedules due at `now`, each marked as run now so it runs once
/// however many times it was missed; the scheduler starts them.
pub fn take_due(now: u64) -> Vec<Schedule> {
    let mut due = Vec::new();
    let _ = dir().and_then(|dir| {
        change(&dir, |list| {
            due = due_in(list, now);
            for schedule in list.iter_mut() {
                if due.iter().any(|run| run.id == schedule.id) {
                    schedule.last_run_ms = Some(now);
                }
            }
            // A one-time schedule is done once it has run.
            list.retain(|schedule| !(schedule.once && schedule.last_run_ms.is_some()));
        })
    });
    due
}

/// Which of `list` are due at `now` (pure).
pub fn due_in(list: &[Schedule], now: u64) -> Vec<Schedule> {
    list.iter()
        .filter(|schedule| schedule.enabled)
        .filter(|schedule| schedule.next_due().is_some_and(|due| due <= now))
        .cloned()
        .collect()
}

fn load(dir: &std::path::Path) -> Vec<Schedule> {
    fs::read(dir.join("schedules.json"))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

/// Changes the list under a lock, so the app and a task's tools agree.
fn change(dir: &std::path::Path, edit: impl FnOnce(&mut Vec<Schedule>)) -> Result<()> {
    use fs2::FileExt;
    crate::app_paths::create_private_dir(dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("schedules.lock"))?;
    lock.lock_exclusive()?;
    let mut list = load(dir);
    edit(&mut list);
    let data = serde_json::to_vec_pretty(&list)?;
    let written = store::write_private(&dir.join("schedules.json"), &data);
    let _ = FileExt::unlock(&lock);
    written.map_err(|error| eyre!("couldn't save the schedules: {error}"))
}

/// When a schedule runs: the local minutes, hours, days, months, and
/// weekdays that match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rule {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    /// `None`: any day of the month.
    month_days: Option<BTreeSet<u32>>,
    months: BTreeSet<u32>,
    /// 0 is Sunday; `None`: any weekday.
    weekdays: Option<BTreeSet<u32>>,
}

impl Rule {
    /// A cron line (`m h dom mon dow`) or an RRULE.
    pub fn parse(when: &str) -> Result<Self> {
        let when = when.trim();
        let upper = when.to_ascii_uppercase();
        if upper.starts_with("RRULE:") || upper.starts_with("FREQ=") {
            return Self::rrule(upper.trim_start_matches("RRULE:"));
        }
        Self::cron(when)
    }

    fn cron(line: &str) -> Result<Self> {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return Err(eyre!(
                "“{line}” isn't a schedule: give a cron line (minute hour day month weekday) or an RRULE"
            ));
        };
        let any = |field: &str| field == "*";
        let weekdays = field(weekday, 0, 7)?
            .into_iter()
            .map(|day| day % 7)
            .collect::<BTreeSet<_>>();
        Ok(Self {
            minutes: field(minute, 0, 59)?,
            hours: field(hour, 0, 23)?,
            month_days: (!any(day)).then(|| field(day, 1, 31)).transpose()?,
            months: field(month, 1, 12)?,
            weekdays: (!any(weekday)).then_some(weekdays),
        })
    }

    fn rrule(text: &str) -> Result<Self> {
        let mut freq = None;
        let mut interval = 1u32;
        let mut by_hour = None;
        let mut by_minute = None;
        let mut by_day = None;
        let mut by_month_day = None;
        for part in text.split(';').filter(|part| !part.is_empty()) {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| eyre!("“{part}” isn't part of an RRULE"))?;
            let numbers = |max: u32| -> Result<BTreeSet<u32>> {
                value
                    .split(',')
                    .map(|n| {
                        n.trim()
                            .parse::<u32>()
                            .ok()
                            .filter(|n| *n <= max)
                            .ok_or_else(|| eyre!("“{value}” isn't valid in {key}"))
                    })
                    .collect()
            };
            match key {
                "FREQ" => freq = Some(value.to_owned()),
                "INTERVAL" => {
                    interval = value
                        .parse()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or_else(|| eyre!("INTERVAL must be a positive number"))?;
                }
                "BYHOUR" => by_hour = Some(numbers(23)?),
                "BYMINUTE" => by_minute = Some(numbers(59)?),
                "BYMONTHDAY" => by_month_day = Some(numbers(31)?),
                "BYDAY" => {
                    const DAYS: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];
                    by_day = Some(
                        value
                            .split(',')
                            .map(|day| {
                                DAYS.iter()
                                    .position(|name| *name == day.trim())
                                    .map(|index| index as u32)
                                    .ok_or_else(|| eyre!("“{day}” isn't a day (MO, TU, …)"))
                            })
                            .collect::<Result<BTreeSet<_>>>()?,
                    );
                }
                // Limits a schedule's own time can't honour are refused, not
                // silently ignored.
                other => return Err(eyre!("Hex's schedules don't support {other}")),
            }
        }
        let every = |step: u32, max: u32| (0..=max).step_by(step as usize).collect();
        let minutes = by_minute.unwrap_or_else(|| BTreeSet::from([0]));
        let all_hours: BTreeSet<u32> = (0..=23).collect();
        let freq = freq.ok_or_else(|| eyre!("an RRULE needs FREQ"))?;
        if interval > 1 && !matches!(freq.as_str(), "MINUTELY" | "HOURLY") {
            return Err(eyre!(
                "INTERVAL works with FREQ=MINUTELY or HOURLY; use BYDAY for weekly patterns"
            ));
        }
        let (minutes, hours) = match freq.as_str() {
            "MINUTELY" => (every(interval, 59), all_hours),
            "HOURLY" => (minutes, by_hour.unwrap_or_else(|| every(interval, 23))),
            "DAILY" | "WEEKLY" | "MONTHLY" => (
                minutes,
                by_hour.ok_or_else(|| eyre!("say when in the day: BYHOUR=8"))?,
            ),
            other => return Err(eyre!("FREQ={other} isn't supported")),
        };
        if freq == "WEEKLY" && by_day.is_none() {
            return Err(eyre!("say which days: BYDAY=MO"));
        }
        if freq == "MONTHLY" && by_month_day.is_none() {
            return Err(eyre!("say which day of the month: BYMONTHDAY=1"));
        }
        Ok(Self {
            minutes,
            hours,
            month_days: by_month_day,
            months: (1..=12).collect(),
            weekdays: by_day,
        })
    }

    fn matches(&self, time: &Local) -> bool {
        let day = match (&self.month_days, &self.weekdays) {
            (None, None) => true,
            (Some(days), None) => days.contains(&time.day),
            (None, Some(weekdays)) => weekdays.contains(&time.weekday),
            // Cron: either matches when both are given.
            (Some(days), Some(weekdays)) => {
                days.contains(&time.day) || weekdays.contains(&time.weekday)
            }
        };
        day && self.months.contains(&time.month)
            && self.hours.contains(&time.hour)
            && self.minutes.contains(&time.minute)
    }

    /// The first matching local minute after `after_ms`.
    pub fn next_after(&self, after_ms: u64) -> Option<u64> {
        self.next_after_in(after_ms, local)
    }

    fn next_after_in(&self, after_ms: u64, to_local: impl Fn(u64) -> Local) -> Option<u64> {
        let mut minute = after_ms / 60_000 + 1;
        let end = minute + HORIZON_MINUTES;
        while minute < end {
            let time = to_local(minute * 60_000);
            if self.matches(&time) {
                return Some(minute * 60_000);
            }
            // Skip what can't match: the rest of the hour, or of the day.
            let day_ok = self.months.contains(&time.month)
                && match (&self.month_days, &self.weekdays) {
                    (None, None) => true,
                    (Some(days), None) => days.contains(&time.day),
                    (None, Some(weekdays)) => weekdays.contains(&time.weekday),
                    (Some(days), Some(weekdays)) => {
                        days.contains(&time.day) || weekdays.contains(&time.weekday)
                    }
                };
            minute += if !day_ok {
                u64::from((23 - time.hour) * 60 + (60 - time.minute))
            } else if !self.hours.contains(&time.hour) {
                u64::from(60 - time.minute)
            } else {
                1
            };
        }
        None
    }
}

/// `field` of a cron line as the numbers it allows: `*`, `*/n`, `a`,
/// `a-b`, `a-b/n`, and lists of them.
fn field(field: &str, min: u32, max: u32) -> Result<BTreeSet<u32>> {
    let mut values = BTreeSet::new();
    for part in field.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (
                range,
                step.parse::<u32>()
                    .ok()
                    .filter(|step| *step > 0)
                    .ok_or_else(|| eyre!("“{part}” has no valid step"))?,
            ),
            None => (part, 1),
        };
        let number = |text: &str| {
            text.parse::<u32>()
                .ok()
                .filter(|n| (min..=max).contains(n))
                .ok_or_else(|| eyre!("“{text}” is out of range ({min}–{max})"))
        };
        let (start, end) = match range {
            "*" => (min, max),
            range => match range.split_once('-') {
                Some((start, end)) => (number(start)?, number(end)?),
                None if step > 1 => (number(range)?, max),
                None => {
                    let n = number(range)?;
                    (n, n)
                }
            },
        };
        if start > end {
            return Err(eyre!("“{part}” runs backwards"));
        }
        values.extend((start..=end).step_by(step as usize));
    }
    Ok(values)
}

/// A local time's parts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Local {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// 0 is Sunday.
    pub weekday: u32,
}

/// `ms` in this computer's time zone.
pub fn local(ms: u64) -> Local {
    match local_tm(ms) {
        Some(tm) => Local {
            year: tm.tm_year + 1900,
            month: tm.tm_mon as u32 + 1,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            weekday: tm.tm_wday as u32,
        },
        None => utc(ms),
    }
}

/// `ms` broken into this computer's local time (`localtime_r`, or
/// `localtime_s` on Windows); `None` when the zone can't be read.
pub(crate) fn local_tm(ms: u64) -> Option<libc::tm> {
    let seconds = (ms / 1000) as libc::time_t;
    // SAFETY: a zeroed `tm` is valid; the call writes only into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    #[cfg(unix)]
    // SAFETY: both pointers are valid for the call.
    let ok = !unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null();
    #[cfg(windows)]
    // SAFETY: both pointers are valid for the call.
    let ok = unsafe { libc::localtime_s(&mut tm, &seconds) } == 0;
    #[cfg(not(any(unix, windows)))]
    let ok = false;
    ok.then_some(tm)
}

/// This computer's offset from UTC at `ms`, in seconds.
pub(crate) fn local_offset(ms: u64) -> i64 {
    let Some(tm) = local_tm(ms) else {
        return 0;
    };
    #[cfg(unix)]
    {
        tm.tm_gmtoff
    }
    // The wall clock read as UTC, less the instant itself.
    #[cfg(not(unix))]
    {
        let days = days_from_civil(
            i64::from(tm.tm_year) + 1900,
            i64::from(tm.tm_mon) + 1,
            i64::from(tm.tm_mday),
        );
        let wall = days * 86_400
            + i64::from(tm.tm_hour) * 3600
            + i64::from(tm.tm_min) * 60
            + i64::from(tm.tm_sec);
        wall - (ms / 1000) as i64
    }
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
#[cfg_attr(unix, allow(dead_code))]
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// `ms` in UTC (tests, and where the zone can't be read).
fn utc(ms: u64) -> Local {
    let minutes = ms / 60_000;
    let days = (minutes / 1440) as i64;
    let (year, month, day) = civil(days);
    Local {
        year: year as i32,
        month,
        day,
        hour: (minutes % 1440 / 60) as u32,
        minute: (minutes % 60) as u32,
        weekday: ((days + 4).rem_euclid(7)) as u32,
    }
}

fn civil(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// `ms` as a person reads it, locally: "Thu 2026-11-13 08:00".
pub fn describe(ms: u64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let time = local(ms);
    format!(
        "{} {:04}-{:02}-{:02} {:02}:{:02}",
        DAYS[time.weekday as usize % 7],
        time.year,
        time.month,
        time.day,
        time.hour,
        time.minute
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_round_trip() {
        for days in [0_i64, 59, 60, 365, 11_016, 20_000, 20_769] {
            let (year, month, day) = civil(days);
            assert_eq!(days_from_civil(year, month as i64, day as i64), days);
        }
    }

    /// Thursday 2026-11-12 09:30 UTC.
    const NOW: u64 = 1_794_475_800_000;

    fn next(when: &str, after: u64) -> Option<Local> {
        Rule::parse(when)
            .unwrap()
            .next_after_in(after, utc)
            .map(utc)
    }

    fn at(day: u32, hour: u32, minute: u32) -> Local {
        let weekday = (4 + day - 12) % 7;
        Local {
            year: 2026,
            month: 11,
            day,
            hour,
            minute,
            weekday,
        }
    }

    #[test]
    fn the_clock_is_what_the_tests_say() {
        assert_eq!(utc(NOW), at(12, 9, 30));
    }

    #[test]
    fn cron_lines_and_rrules_find_their_next_time() {
        // Every morning at 8: tomorrow.
        assert_eq!(next("0 8 * * *", NOW), Some(at(13, 8, 0)));
        assert_eq!(
            next("FREQ=DAILY;BYHOUR=8;BYMINUTE=0", NOW),
            Some(at(13, 8, 0))
        );
        // Weekdays at 8: Friday, then Monday.
        assert_eq!(next("0 8 * * 1-5", NOW), Some(at(13, 8, 0)));
        assert_eq!(next("0 8 * * 1-5", NOW + 86_400_000), Some(at(16, 8, 0)));
        assert_eq!(
            next("RRULE:FREQ=WEEKLY;BYDAY=MO;BYHOUR=9", NOW),
            Some(at(16, 9, 0))
        );
        // Every 15 minutes, every 2 hours.
        assert_eq!(next("*/15 * * * *", NOW), Some(at(12, 9, 45)));
        assert_eq!(next("FREQ=MINUTELY;INTERVAL=15", NOW), Some(at(12, 9, 45)));
        assert_eq!(next("FREQ=HOURLY;INTERVAL=2", NOW), Some(at(12, 10, 0)));
        // The first of the month.
        assert_eq!(
            next("FREQ=MONTHLY;BYMONTHDAY=1;BYHOUR=9", NOW).map(|t| (t.month, t.day)),
            Some((12, 1))
        );
        // A minute from now, today.
        assert_eq!(next("31 9 12 11 *", NOW), Some(at(12, 9, 31)));
    }

    #[test]
    fn bad_schedules_say_why() {
        for bad in [
            "every morning",
            "61 8 * * *",
            "0 8 * *",
            "FREQ=DAILY",
            "FREQ=WEEKLY;BYHOUR=9",
            "FREQ=DAILY;INTERVAL=2;BYHOUR=8",
            "FREQ=YEARLY;BYHOUR=8",
            "FREQ=DAILY;BYHOUR=8;COUNT=3",
        ] {
            assert!(Rule::parse(bad).is_err(), "{bad}");
        }
        // 31 February never comes.
        assert_eq!(
            Rule::parse("0 8 31 2 *").unwrap().next_after_in(NOW, utc),
            None
        );
    }

    #[test]
    fn missed_runs_run_once() {
        let schedule = |last_run_ms| Schedule {
            id: "s1".into(),
            when: "*/15 * * * *".into(),
            task: "check".into(),
            lane: Lane::Background,
            enabled: true,
            created_ms: NOW - 3 * 3_600_000,
            last_run_ms,
            once: false,
        };
        // Three hours asleep: one run, not twelve.
        let list = [schedule(None)];
        assert_eq!(due_in(&list, NOW).len(), 1);
        // Once run, it waits for its next time.
        assert!(due_in(&[schedule(Some(NOW))], NOW + 60_000).is_empty());
        assert_eq!(due_in(&[schedule(Some(NOW))], NOW + 15 * 60_000).len(), 1);
        let mut off = schedule(None);
        off.enabled = false;
        assert!(due_in(&[off], NOW).is_empty());
    }
}
