//! Usage limits and rate limits, read from each backend's own words, and
//! when they lift. The shapes come from real runs:
//!
//! - Claude Code: "You've hit your session limit · resets 2am (Europe/Berlin)",
//!   "You've hit your weekly limit · resets Oct 22 at 1pm (Europe/Berlin)",
//!   older builds "Claude AI usage limit reached|1760000000", and
//!   "API Error: 429 …" / "529 Overloaded".
//! - Codex: "You've hit your usage limit. … or try again at 3:04 PM." (or
//!   "at Nov 13th, 2026 3:04 PM", or "Try again later.").
//! - OpenCode: a provider error such as "Rate limit exceeded" with status 429.
//! - The Claude API: HTTP 429 with `retry-after` seconds.
//!
//! A reset time with a zone name ("Europe/Berlin") is read as this Mac's
//! local time: the agents print the person's own zone.

use std::sync::OnceLock;

use regex::Regex;

/// A backend refused because of a limit.
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    /// The backend's words, for a person.
    pub reason: String,
    /// When it lifts (Unix ms), if the backend said.
    pub until_ms: Option<u64>,
}

/// The limit `message` reports, now, on this Mac's clock and zone.
pub fn detect(message: &str) -> Option<Limit> {
    detect_at(message, super::super::store::now_ms(), local_offset_seconds())
}

/// The limit `message` reports, `now_ms` being now and `offset_s` this
/// Mac's offset from UTC.
pub fn detect_at(message: &str, now_ms: u64, offset_s: i64) -> Option<Limit> {
    if !is_limit(message) {
        return None;
    }
    let until_ms = epoch_reset(message)
        .or_else(|| clock_reset(message, now_ms, offset_s))
        .or_else(|| relative_reset(message, now_ms))
        .filter(|until| *until > now_ms && *until < now_ms + 8 * DAY_MS);
    Some(Limit {
        reason: reason(message),
        until_ms,
    })
}

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn is_limit(message: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            Regex::new(
                r"(?i)usage limit|session limit|weekly limit|hit your (\w+ )?limit|limit reached|rate[ _-]?limit|too many requests|\b429\b|\b529\b|overloaded|quota exceeded|resource_exhausted",
            )
            .expect("valid pattern")
        })
        .is_match(message)
}

/// "usage limit reached|1760000000": Unix seconds.
fn epoch_reset(message: &str) -> Option<u64> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern =
        PATTERN.get_or_init(|| Regex::new(r"limit reached\|(\d{10})\b").expect("valid pattern"));
    let seconds: u64 = pattern.captures(message)?[1].parse().ok()?;
    Some(seconds * 1000)
}

/// "resets 2am", "resets Oct 22 at 1pm", "resets 10:50pm", "try again at
/// 3:04 PM", "try again at Nov 13th, 2026 3:04 PM": the next such local time.
fn clock_reset(message: &str, now_ms: u64, offset_s: i64) -> Option<u64> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r"(?i)(?:resets?|try again at|available again at)\s+(?:at\s+)?(?:(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s+(?:(\d{4})\s+)?(?:at\s+)?)?(\d{1,2})(?::(\d{2}))?\s*([ap])?\.?m?\.?",
        )
        .expect("valid pattern")
    });
    let captures = pattern.captures(message)?;
    let minute: i64 = captures.get(5).map_or(Ok(0), |m| m.as_str().parse()).ok()?;
    let mut hour: i64 = captures[4].parse().ok()?;
    match captures.get(6).map(|m| m.as_str().to_ascii_lowercase()) {
        Some(half) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            hour %= 12;
            if half == "p" {
                hour += 12;
            }
        }
        // A bare number is only a time with minutes ("resets 14:30").
        None if captures.get(5).is_none() => return None,
        None => {}
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    let now_local = now_ms as i64 / 1000 + offset_s;
    let today = now_local.div_euclid(86_400);
    let (year, month, day) = civil_from_days(today);
    let second_of_day = hour * 3600 + minute * 60;
    let local = match captures.get(1) {
        Some(name) => {
            let month = month_number(name.as_str())?;
            let day: i64 = captures[2].parse().ok()?;
            let explicit = captures.get(3).and_then(|y| y.as_str().parse().ok());
            let mut at = days_from_civil(explicit.unwrap_or(year), month, day) * 86_400
                + second_of_day;
            if explicit.is_none() && at < now_local - 86_400 {
                at = days_from_civil(year + 1, month, day) * 86_400 + second_of_day;
            }
            at
        }
        None => {
            let mut at = days_from_civil(year, month, day) * 86_400 + second_of_day;
            if at <= now_local {
                at += 86_400;
            }
            at
        }
    };
    u64::try_from((local - offset_s) * 1000).ok()
}

/// "try again in 2 hours 5 minutes", "retry after 30 seconds", "retry-after: 20".
fn relative_reset(message: &str, now_ms: u64) -> Option<u64> {
    static START: OnceLock<Regex> = OnceLock::new();
    static AMOUNT: OnceLock<Regex> = OnceLock::new();
    let start = START.get_or_init(|| {
        Regex::new(r"(?i)(?:try again in|retry (?:in|after)|retry-after:?)\s*")
            .expect("valid pattern")
    });
    let amount = AMOUNT.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:and\s+)?(\d+(?:\.\d+)?)\s*(ms|milliseconds?|s\b|secs?|seconds?|m\b|mins?|minutes?|h\b|hrs?|hours?)?,?")
            .expect("valid pattern")
    });
    let found = start.find(message)?;
    let mut rest = &message[found.end()..];
    let mut total = 0f64;
    let mut any = false;
    while let Some(captures) = amount.captures(rest) {
        let value: f64 = captures[1].parse().ok()?;
        let unit = captures
            .get(2)
            .map_or("s", |m| m.as_str())
            .to_ascii_lowercase();
        total += value
            * if unit.starts_with("ms") || unit.starts_with("milli") {
                1.0
            } else if unit.starts_with('h') {
                3_600_000.0
            } else if unit.starts_with('m') {
                60_000.0
            } else {
                1000.0
            };
        any = true;
        rest = &rest[captures[0].len()..];
        if captures.get(2).is_none() {
            break;
        }
    }
    any.then(|| now_ms + total as u64)
}

fn reason(message: &str) -> String {
    let line = message
        .lines()
        .map(str::trim)
        .find(|line| is_limit(line))
        .unwrap_or_else(|| message.trim());
    let line = line.split('|').next().unwrap_or(line).trim();
    super::super::store::shorten(line, 200)
}

fn month_number(name: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let name = name.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|month| name.starts_with(month))
        .map(|index| index as i64 + 1)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// This Mac's offset from UTC now, in seconds.
fn local_offset_seconds() -> i64 {
    #[cfg(unix)]
    {
        let seconds = (super::super::store::now_ms() / 1000) as libc::time_t;
        // SAFETY: a zeroed `tm` is valid; `localtime_r` writes only into it.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: both pointers are valid for the call.
        if unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
            return 0;
        }
        tm.tm_gmtoff
    }
    #[cfg(not(unix))]
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-12 09:30 in Berlin (UTC+1).
    const NOW: u64 = 1_794_472_200_000;
    const BERLIN: i64 = 3600;

    fn at(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> u64 {
        ((days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 - BERLIN) * 1000)
            as u64
    }

    #[test]
    fn the_clock_for_the_tests_is_what_it_says() {
        assert_eq!(at(2026, 11, 12, 9, 30), NOW);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn recorded_claude_code_limits_name_their_reset() {
        let fixtures = include_str!("../../../tests/fixtures/assistant/limits.txt");
        let mut seen = 0;
        for line in fixtures.lines().filter(|line| !line.trim().is_empty()) {
            let (expected, message) = line.split_once(" => ").unwrap();
            let limit = detect_at(message, NOW, BERLIN).unwrap_or_else(|| panic!("{message}"));
            let expected = match expected.trim() {
                "none" => None,
                time => {
                    let numbers: Vec<i64> = time
                        .split(['-', ' ', ':'])
                        .map(|n| n.parse().unwrap())
                        .collect();
                    Some(at(numbers[0], numbers[1], numbers[2], numbers[3], numbers[4]))
                }
            };
            assert_eq!(limit.until_ms, expected, "{message}");
            assert!(!limit.reason.is_empty());
            seen += 1;
        }
        assert!(seen >= 8);
    }

    #[test]
    fn ordinary_failures_are_not_limits() {
        for message in [
            "Claude Code: No conversation found with session ID: x",
            "OpenCode: no credentials",
            "Login expired · Please run /login",
            "Hex can't save conversations",
        ] {
            assert_eq!(detect_at(message, NOW, BERLIN), None, "{message}");
        }
    }

    #[test]
    fn relative_waits_and_epochs_are_read() {
        let limit = detect_at(
            "The Claude API is rate-limiting this key (429); retry after 30 seconds.",
            NOW,
            BERLIN,
        )
        .unwrap();
        assert_eq!(limit.until_ms, Some(NOW + 30_000));
        let limit = detect_at("Claude AI usage limit reached|1794480000", NOW, BERLIN).unwrap();
        assert_eq!(limit.until_ms, Some(1_794_480_000_000));
        assert_eq!(limit.reason, "Claude AI usage limit reached");
        let limit = detect_at(
            "Rate limit reached. Please try again in 2 hours 5 minutes.",
            NOW,
            BERLIN,
        )
        .unwrap();
        assert_eq!(limit.until_ms, Some(NOW + (2 * 60 + 5) * 60_000));
    }
}
