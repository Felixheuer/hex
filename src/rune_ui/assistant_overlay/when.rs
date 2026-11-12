//! When a task waiting out a usage limit resumes: the local time of day
//! ("14:05") and the minutes left, for its row, its bubble, and its outline.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Now, in Unix milliseconds.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// `at_ms` (Unix milliseconds) as the local 24-hour time of day, "14:05".
pub(crate) fn clock(at_ms: u64) -> String {
    let (hour, minute) = local_hour_minute(at_ms);
    hh_mm(hour, minute)
}

fn hh_mm(hour: u32, minute: u32) -> String {
    format!("{hour:02}:{minute:02}")
}

#[cfg(unix)]
fn local_hour_minute(at_ms: u64) -> (u32, u32) {
    let seconds = (at_ms / 1_000) as libc::time_t;
    // SAFETY: `localtime_r` writes only into the provided zeroed `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&seconds, &mut tm) };
    (tm.tm_hour as u32, tm.tm_min as u32)
}

#[cfg(not(unix))]
fn local_hour_minute(at_ms: u64) -> (u32, u32) {
    let minutes = at_ms / 60_000;
    (((minutes / 60) % 24) as u32, (minutes % 60) as u32)
}

/// Whole minutes left until `at_ms`, rounded up (at least 1 while any time
/// is left, 0 once it's passed).
pub(crate) fn minutes_left(at_ms: u64, now_ms: u64) -> u64 {
    at_ms.saturating_sub(now_ms).div_ceil(60_000)
}

/// How long until the minutes left next change, so a countdown redraws
/// once a minute rather than every frame.
pub(crate) fn next_tick(at_ms: u64, now_ms: u64) -> Option<Duration> {
    let left = at_ms.checked_sub(now_ms).filter(|left| *left > 0)?;
    let into = left % 60_000;
    Some(Duration::from_millis(if into == 0 { 60_000 } else { into }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_countdown_rounds_up_and_ticks_on_the_minute() {
        assert_eq!(hh_mm(9, 5), "09:05");
        assert_eq!(hh_mm(14, 30), "14:30");
        let now = 1_000_000;
        assert_eq!(minutes_left(now + 1, now), 1);
        assert_eq!(minutes_left(now + 60_000, now), 1);
        assert_eq!(minutes_left(now + 60_001, now), 2);
        assert_eq!(minutes_left(now - 5, now), 0);
        assert_eq!(
            next_tick(now + 90_000, now),
            Some(Duration::from_millis(30_000))
        );
        assert_eq!(
            next_tick(now + 120_000, now),
            Some(Duration::from_millis(60_000))
        );
        assert_eq!(next_tick(now, now), None);
        assert_eq!(clock(now).len(), 5);
    }
}
