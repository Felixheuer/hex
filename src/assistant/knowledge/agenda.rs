//! Calendar events (through Hex's EventKit bridge, `crate::calendar`) and
//! open reminders (`knowledge.swift`): titles, times, places, who else
//! comes; never an invitation's notes.

use serde::Deserialize;

use super::native::{self, Access};
use super::rank::{self, Hit};

const DAY_MS: i64 = 86_400_000;
/// How far back and ahead a lookup reads.
const BACK_DAYS: i64 = 30;
const AHEAD_DAYS: i64 = 90;

/// One event as it ranks: what, when, where, with whom.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub title: String,
    pub start_ms: i64,
    pub all_day: bool,
    pub location: Option<String>,
    pub attendees: Vec<String>,
    pub cancelled: bool,
}

/// Events matching the query in title, place, or people, the next ones
/// first, then the latest past ones. No query: what comes next.
pub fn rank_events(events: &[Event], tokens: &[String], now_ms: i64, limit: usize) -> Vec<Hit> {
    let hits = events
        .iter()
        .filter(|event| !event.cancelled)
        .filter_map(|event| {
            let text = format!(
                "{} {} {}",
                event.title,
                event.location.as_deref().unwrap_or_default(),
                event.attendees.join(" ")
            );
            let matched = rank::score(&text, tokens);
            if matched <= 0.0 || (tokens.is_empty() && event.start_ms < now_ms - DAY_MS / 2) {
                return None;
            }
            // Upcoming beats past; nearer beats farther.
            let ahead = event.start_ms >= now_ms - DAY_MS / 2;
            let distance = ((event.start_ms - now_ms).abs() as f64) / DAY_MS as f64;
            let score = matched + if ahead { 0.5 } else { 0.0 } + 0.3 / (1.0 + distance);
            let mut detail = Vec::new();
            if event.all_day {
                detail.push("all day".to_owned());
            }
            if let Some(place) = event.location.as_deref().filter(|place| !place.is_empty()) {
                detail.push(crate::assistant::store::shorten(place, 80));
            }
            if !event.attendees.is_empty() {
                let mut people = event.attendees.iter().take(5).cloned().collect::<Vec<_>>();
                if event.attendees.len() > 5 {
                    people.push(format!("+{}", event.attendees.len() - 5));
                }
                detail.push(format!("with {}", people.join(", ")));
            }
            Some(Hit {
                title: event.title.clone(),
                detail: detail.join(" · "),
                when_ms: Some(event.start_ms),
                score,
            })
        })
        .collect();
    rank::top(hits, limit)
}

#[cfg(target_os = "macos")]
fn read_events(now_ms: i64) -> Result<Vec<Event>, String> {
    let access = crate::calendar::access();
    if access != crate::calendar::CalendarAccess::FullAccess {
        return Err("Hex has no calendar access (Settings › Meetings › Connect calendar, or System Settings › Privacy & Security › Calendars)".into());
    }
    // Two reads, so a busy past month can't crowd out the months ahead
    // (each read is capped).
    let mut events = crate::calendar::events(now_ms - BACK_DAYS * DAY_MS, now_ms)
        .map_err(|error| error.to_string())?;
    events.extend(
        crate::calendar::events(now_ms, now_ms + AHEAD_DAYS * DAY_MS)
            .map_err(|error| error.to_string())?,
    );
    Ok(events
        .into_iter()
        .map(|event| Event {
            title: event.title,
            start_ms: event.start_ms as i64,
            all_day: event.all_day,
            location: event.location,
            attendees: event.attendees,
            cancelled: event.cancelled || event.declined,
        })
        .collect())
}

#[cfg(not(target_os = "macos"))]
fn read_events(_: i64) -> Result<Vec<Event>, String> {
    Err("calendar lookups need macOS".into())
}

pub fn calendar(tokens: &[String], limit: usize, now_ms: i64) -> Result<Vec<Hit>, String> {
    Ok(rank_events(&read_events(now_ms)?, tokens, now_ms, limit))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reminder {
    title: String,
    list: String,
    due_ms: Option<f64>,
    priority: i64,
}

/// Open reminders matching the query (in their title or list), due first.
pub fn reminders(query: &str, tokens: &[String], limit: usize) -> Result<Vec<Hit>, String> {
    match native::reminders_access() {
        Access::Granted => {}
        Access::NotAsked => {
            return Err("Hex hasn't asked for Reminders access yet (System Settings › Privacy & Security › Reminders)".into());
        }
        Access::Denied => {
            return Err("Reminders access is off for Hex (System Settings › Privacy & Security › Reminders)".into());
        }
    }
    let json = native::reminders(query, 300).ok_or("Reminders didn't answer in time")?;
    let found: Vec<Reminder> = serde_json::from_str(&json).map_err(|error| error.to_string())?;
    let hits = found
        .into_iter()
        .map(|reminder| {
            let score = rank::score(&format!("{} {}", reminder.title, reminder.list), tokens)
                + if reminder.due_ms.is_some() { 0.2 } else { 0.0 }
                + if reminder.priority > 0 { 0.1 } else { 0.0 };
            Hit {
                detail: format!("list {}", reminder.list),
                title: reminder.title,
                when_ms: reminder.due_ms.map(|ms| ms as i64),
                score,
            }
        })
        .collect();
    Ok(rank::top(hits, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_matching_event_comes_first() {
        let now = 1_000 * DAY_MS;
        let event = |title: &str, start_ms, cancelled| Event {
            title: title.into(),
            start_ms,
            all_day: false,
            location: Some("Büro".into()),
            attendees: vec!["Michael Schwarz".into()],
            cancelled,
        };
        let events = [
            event("Kalibermarkt Sync", now - 3 * DAY_MS, false),
            event("Kalibermarkt Sync", now + 2 * DAY_MS, false),
            event("Kalibermarkt Review", now + DAY_MS, true),
            event("Zahnarzt", now + DAY_MS, false),
        ];
        let hits = rank_events(&events, &rank::tokens("kalibermarkt"), now, 5);
        assert_eq!(hits.len(), 2, "cancelled and unrelated left out");
        assert_eq!(hits[0].when_ms, Some(now + 2 * DAY_MS));
        assert_eq!(hits[0].detail, "Büro · with Michael Schwarz");
        assert_eq!(
            rank_events(&events, &rank::tokens("schwarz"), now, 5).len(),
            3,
            "every event he comes to, not the cancelled one"
        );
        let next = rank_events(&events, &[], now, 5);
        assert_eq!(next[0].title, "Zahnarzt", "no query: the next one");
        assert!(next.iter().all(|hit| hit.when_ms >= Some(now - DAY_MS)));
    }
}
