//! `hex_calendar_write`: the person's own appointments and reminders, added,
//! changed, ticked off, or deleted straight in Calendar or Reminders through
//! EventKit (`knowledge.swift`),
//! without opening either app and without Automation consent. Only with the
//! access onboarding already got; a tool call never asks.

use serde::Serialize;
use serde_json::{Value, json};

pub const NAME: &str = "hex_calendar_write";

pub fn definition() -> Value {
    json!({
        "name": NAME,
        "description": "Write the person's own appointments (kind event, Calendar) and reminders (kind reminder, Reminders) directly and in the background, without opening either app: action add (default), change (only the fields given; moving the start keeps the length), done (a reminder ticked off), or delete (only when the person asks). change, done, and delete take the id hex_lookup shows (scope calendar or reminders). Times are local: \"2026-11-16 15:00\", or a date alone for all day or a reminder without a time. Answers with the calendar or list and the times as saved; check them against the request.",
        "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["add", "change", "done", "delete"]},
            "id": {"type": "string", "description": "change, done, delete: the event's or reminder's id from hex_lookup."},
            "kind": {"type": "string", "enum": ["event", "reminder"]},
            "title": {"type": "string"},
            "start": {"type": "string", "description": "event: its start; reminder: when it is due (optional). \"YYYY-MM-DD HH:MM\" or \"YYYY-MM-DD\", local time."},
            "end": {"type": "string", "description": "event: its end (default: start + duration_minutes, else one hour; all-day: that day)."},
            "duration_minutes": {"type": "integer", "minimum": 1},
            "alert_minutes": {"type": "integer", "minimum": 0, "description": "An alert this many minutes before (0: at the time)."},
            "list": {"type": "string", "description": "The calendar or reminder list's name (default: the person's default one)."},
            "location": {"type": "string"},
            "notes": {"type": "string"}
        }, "required": ["kind"]},
    })
}

/// A local date, with a time or without.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct When {
    year: i32,
    month: u32,
    day: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    hour: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minute: Option<u32>,
}

/// "2026-11-16", "2026-11-16 15:00", "2026-11-16T15:00:00", "16.11.2026 15:00".
pub fn parse_when(text: &str) -> Result<When, String> {
    let text = text.trim();
    let invalid =
        || format!("“{text}” isn't a local date and time like 2026-11-16 15:00 or 2026-11-16");
    let (date, time) = match text.split_once(['T', ' ']) {
        Some((date, time)) => (date.trim(), Some(time.trim())),
        None => (text, None),
    };
    let number = |part: &str| part.trim().parse::<u32>().map_err(|_| invalid());
    let parts: Vec<&str> = date.split(['-', '.']).collect();
    let [first, second, third] = parts.as_slice() else {
        return Err(invalid());
    };
    let (year, month, day) = if date.contains('-') {
        (number(first)?, number(second)?, number(third)?)
    } else {
        (number(third)?, number(second)?, number(first)?)
    };
    let (hour, minute) = match time.filter(|time| !time.is_empty()) {
        None => (None, None),
        Some(time) => {
            let time = time.trim_end_matches(['Z', 'z']);
            let mut fields = time.split(':');
            let hour = number(fields.next().unwrap_or_default())?;
            let minute = fields.next().map_or(Ok(0), number)?;
            (Some(hour), Some(minute))
        }
    };
    let days = match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => 0,
    };
    if !(1000..=9999).contains(&year)
        || day == 0
        || day > days
        || hour.is_some_and(|hour| hour > 23)
        || minute.is_some_and(|minute| minute > 59)
    {
        return Err(invalid());
    }
    Ok(When {
        year: year as i32,
        month,
        day,
        hour,
        minute,
    })
}

/// The JSON `hex_knowledge_add` reads.
pub fn request(arguments: &Value) -> Result<Value, String> {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let kind = match text("kind") {
        Some(kind @ ("event" | "reminder")) => kind,
        _ => return Err(format!("{NAME} needs “kind”: event or reminder")),
    };
    let action = match text("action").unwrap_or("add") {
        action @ ("add" | "change" | "done" | "delete") => action,
        other => {
            return Err(format!(
                "{NAME} has no action “{other}”; use add, change, done, or delete"
            ));
        }
    };
    if action == "done" && kind == "event" {
        return Err("only a reminder is ticked off (done); an event is changed or deleted".into());
    }
    let id = text("id");
    if action != "add" && id.is_none() {
        return Err(format!(
            "{NAME} needs the “id” hex_lookup shows to {action} an {kind}"
        ));
    }
    let title = text("title");
    if action == "add" && title.is_none() {
        return Err(format!("{NAME} needs a “title”"));
    }
    let start = text("start").map(parse_when).transpose()?;
    let end = text("end").map(parse_when).transpose()?;
    if action == "add" && kind == "event" && start.is_none() {
        return Err(format!("{NAME} needs a “start” for an event"));
    }
    if let (Some(start), Some(end)) = (start, end)
        && start.hour.is_some() != end.hour.is_some()
    {
        return Err("start and end both need a time, or both none (all day)".into());
    }
    let minutes = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_u64)
            .map(|minutes| minutes.min(60 * 24 * 365))
    };
    Ok(json!({
        "action": if action == "done" { "change" } else { action },
        "id": id,
        "completed": (action == "done").then_some(true),
        "kind": kind,
        "title": title,
        "notes": text("notes"),
        "list": text("list"),
        "location": text("location"),
        "start": start,
        "end": end,
        "durationMinutes": minutes("duration_minutes").filter(|minutes| *minutes > 0),
        "alertMinutes": minutes("alert_minutes"),
    }))
}

/// EventKit's answer as the tool's text.
pub fn answer(request: &Value, reply: &Value) -> Result<String, String> {
    let asked = |key: &str| request.get(key).and_then(Value::as_str).unwrap_or_default();
    let field = |key: &str| reply.get(key).and_then(Value::as_str).unwrap_or_default();
    let title = match field("title") {
        "" => asked("title"),
        saved => saved,
    };
    let what = asked("kind");
    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
        let why = field("error");
        let why = if why.is_empty() {
            "EventKit didn't save it"
        } else {
            why
        };
        return Err(format!("Couldn't write the {what} “{title}”: {why}"));
    }
    let done = request.get("completed").and_then(Value::as_bool) == Some(true);
    let mut text = match asked("action") {
        "delete" => format!("Deleted the {what} “{title}” from {}", field("calendar")),
        "change" if done => format!("Ticked off the reminder “{title}” in {}", field("calendar")),
        "change" => format!("Changed the {what} “{title}” in {}", field("calendar")),
        _ => format!("Added the {what} “{title}” to {}", field("calendar")),
    };
    if asked("action") != "delete" {
        match (field("start"), field("end")) {
            ("", _) => {}
            (start, "") => text.push_str(&format!(", {start}")),
            (start, end) => text.push_str(&format!(", {start} – {end}")),
        }
        if !field("id").is_empty() {
            text.push_str(&format!(" (id {})", field("id")));
        }
    }
    text.push('.');
    Ok(text)
}

pub fn call(arguments: &Value) -> Result<String, String> {
    let request = request(arguments)?;
    let reply = super::native::add(&request.to_string())
        .ok_or("Calendar and Reminders can only be written on macOS")?;
    let reply: Value = serde_json::from_str(&reply).map_err(|error| error.to_string())?;
    answer(&request, &reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_times_read_in_common_forms() {
        let at = |hour, minute| When {
            year: 2026,
            month: 11,
            day: 16,
            hour,
            minute,
        };
        assert_eq!(parse_when("2026-11-16 15:00"), Ok(at(Some(15), Some(0))));
        assert_eq!(
            parse_when("2026-11-16T15:30:00"),
            Ok(at(Some(15), Some(30)))
        );
        assert_eq!(parse_when("16.11.2026 15:30"), Ok(at(Some(15), Some(30))));
        assert_eq!(parse_when("2026-11-16"), Ok(at(None, None)));
        assert_eq!(parse_when("2028-02-29").map(|when| when.day), Ok(29));
        for bad in [
            "morgen",
            "2026-02-29",
            "2026-11-16 25:00",
            "2026-13-01",
            "16.11.",
        ] {
            assert!(parse_when(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn requests_carry_what_eventkit_needs() {
        let event = request(&json!({"kind": "event", "title": " Zahnarzt ",
            "start": "2026-11-16 15:00", "duration_minutes": 45, "alert_minutes": 30,
            "list": "Privat"}))
        .unwrap();
        assert_eq!(event["title"], "Zahnarzt");
        assert_eq!(
            event["start"],
            json!({"year": 2026, "month": 11, "day": 16, "hour": 15, "minute": 0})
        );
        assert_eq!(event["durationMinutes"], 45);
        assert_eq!(event["alertMinutes"], 30);
        assert_eq!(event["end"], Value::Null);
        let reminder = request(&json!({"kind": "reminder", "title": "Milch"})).unwrap();
        assert_eq!(reminder["start"], Value::Null);
        assert!(request(&json!({"kind": "event", "title": "x"})).is_err());
        assert!(request(&json!({"kind": "note", "title": "x"})).is_err());
        assert!(
            request(
                &json!({"kind": "event", "title": "x", "start": "2026-11-16 15:00",
                "end": "2026-11-17"})
            )
            .is_err()
        );
    }

    #[test]
    fn changes_need_an_id_and_done_is_for_reminders() {
        let change = request(&json!({"action": "change", "kind": "event", "id": "E1",
            "start": "2026-11-16 16:00"}))
        .unwrap();
        assert_eq!(change["action"], "change");
        assert_eq!(change["title"], Value::Null, "only the fields given change");
        let done = request(&json!({"action": "done", "kind": "reminder", "id": "R1"})).unwrap();
        assert_eq!(
            (done["action"].as_str(), done["completed"].as_bool()),
            (Some("change"), Some(true))
        );
        assert!(request(&json!({"action": "delete", "kind": "event"})).is_err());
        assert!(request(&json!({"action": "done", "kind": "event", "id": "E1"})).is_err());
        assert!(request(&json!({"action": "move", "kind": "event", "id": "E1"})).is_err());
    }

    #[test]
    fn answers_say_where_it_went() {
        let add =
            request(&json!({"kind": "event", "title": "Zahnarzt", "start": "2026-11-16 15:00"}))
                .unwrap();
        let saved = json!({"ok": true, "calendar": "Privat", "id": "E1", "title": "Zahnarzt",
            "start": "2026-11-16 15:00", "end": "2026-11-16 15:45"});
        assert_eq!(
            answer(&add, &saved).unwrap(),
            "Added the event “Zahnarzt” to Privat, 2026-11-16 15:00 – 2026-11-16 15:45 (id E1)."
        );
        let milk = request(&json!({"kind": "reminder", "title": "Milch"})).unwrap();
        assert_eq!(
            answer(&milk, &json!({"ok": true, "calendar": "Einkauf"})).unwrap(),
            "Added the reminder “Milch” to Einkauf."
        );
        let done = request(&json!({"action": "done", "kind": "reminder", "id": "R1"})).unwrap();
        assert_eq!(
            answer(
                &done,
                &json!({"ok": true, "calendar": "Einkauf", "title": "Milch"})
            )
            .unwrap(),
            "Ticked off the reminder “Milch” in Einkauf."
        );
        let delete = request(&json!({"action": "delete", "kind": "event", "id": "E1"})).unwrap();
        assert_eq!(
            answer(
                &delete,
                &json!({"ok": true, "calendar": "Privat", "title": "Zahnarzt", "id": "E1"})
            )
            .unwrap(),
            "Deleted the event “Zahnarzt” from Privat."
        );
        let refused = answer(&add, &json!({"ok": false, "error": "no access"}));
        assert_eq!(
            refused.unwrap_err(),
            "Couldn't write the event “Zahnarzt”: no access"
        );
    }
}
