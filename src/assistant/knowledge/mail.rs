//! Mail's own index (`~/Library/Mail/V*/MailData/Envelope Index`), read
//! from a copy with Full Disk Access: subject, sender, date, and mailbox
//! only, never a message's body or summary.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use super::exec;
use super::rank::{self, Hit};

/// The newest Mail data folder's index ("V10" beats "V9").
fn index() -> Option<PathBuf> {
    let mail = crate::app_paths::home_dir()?.join("Library/Mail");
    let newest = std::fs::read_dir(&mail)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let version = name.strip_prefix('V')?.parse::<u32>().ok()?;
            Some((version, entry.path()))
        })
        .max_by_key(|(version, _)| *version)?;
    Some(newest.1.join("MailData/Envelope Index")).filter(|path| path.exists())
}

/// Every word (lowercase, unfolded: SQLite compares them) in the subject,
/// the sender's address, or their name; the newest 200 such messages. No
/// words: the newest messages.
pub fn sql(words: &[String]) -> String {
    let conditions: Vec<String> = words
        .iter()
        .map(|token| {
            format!(
                "({} OR {} OR {})",
                exec::like("s.subject", token),
                exec::like("a.address", token),
                exec::like("a.comment", token)
            )
        })
        .collect();
    let mut sql = String::from(
        "SELECT s.subject AS subject, a.address AS address, a.comment AS name, \
         m.date_received AS received, mb.url AS mailbox \
         FROM messages m \
         JOIN subjects s ON s.ROWID = m.subject \
         LEFT JOIN addresses a ON a.ROWID = m.sender \
         LEFT JOIN mailboxes mb ON mb.ROWID = m.mailbox \
         WHERE m.deleted = 0",
    );
    for condition in conditions {
        sql.push_str(" AND ");
        sql.push_str(&condition);
    }
    sql.push_str(" ORDER BY m.date_received DESC LIMIT 200;");
    sql
}

/// A mailbox URL's last part, readable ("imap://…/INBOX" → "INBOX",
/// "…/%5BGmail%5D/Gesendet" → "Gesendet").
fn mailbox(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or(url);
    percent_decode(last)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            out.push(high * 16 + low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Rows as ranked results; the query in the subject counts a little more
/// than in the sender, and newer mail more than older.
pub fn rank_rows(rows: &[Value], tokens: &[String], now_ms: i64, limit: usize) -> Vec<Hit> {
    let text = |row: &Value, key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let hits = rows
        .iter()
        .map(|row| {
            let subject = text(row, "subject");
            let address = text(row, "address");
            let name = text(row, "name");
            let when = row
                .get("received")
                .and_then(Value::as_i64)
                .map(|seconds| seconds * 1000);
            let matched = rank::score(&subject, tokens)
                .max(rank::score(&format!("{subject} {name} {address}"), tokens) * 0.9)
                .max(0.05);
            let from = match (name.is_empty(), address.is_empty()) {
                (false, false) => format!("from {name} <{address}>"),
                (true, false) => format!("from {address}"),
                (false, true) => format!("from {name}"),
                (true, true) => String::new(),
            };
            let place = mailbox(&text(row, "mailbox"));
            let detail = [from, place]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            Hit {
                title: if subject.is_empty() {
                    "(no subject)".into()
                } else {
                    crate::assistant::store::shorten(&subject, 120)
                },
                detail,
                when_ms: when,
                score: matched + rank::recency(when, now_ms, 0.4),
            }
        })
        .collect();
    rank::top(hits, limit)
}

pub fn mail(query: &str, limit: usize, now_ms: i64) -> Result<Vec<Hit>, String> {
    let tokens = &rank::tokens(query);
    let index = index().ok_or(
        "Mail's index wasn't found or can't be read (Mail set up? Agent-Desk needs Full Disk Access)",
    )?;
    let rows = exec::query_copy(&index, &sql(&rank::words(query)), Duration::from_secs(4))?;
    Ok(rank_rows(&rows, tokens, now_ms, limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_query_reads_envelopes_only() {
        let sql = sql(&rank::words("Rechnung Schwarz"));
        assert!(sql.contains("s.subject LIKE '%rechnung%'"));
        assert!(sql.contains("a.comment LIKE '%schwarz%'"));
        assert_eq!(sql.matches(" AND (").count(), 2);
        for body in ["summary", "summaries", "body", "snippet"] {
            assert!(!sql.contains(body), "{body}");
        }
        assert!(!super::sql(&[]).contains(" AND ("));
    }

    #[test]
    fn envelopes_rank_and_read_well() {
        let rows = [
            json!({"subject": "Rechnung 1043", "address": "buchhaltung@x.de", "name": "Buchhaltung",
                   "received": 1_700_000_000, "mailbox": "imap://a%40b@imap.x.de/INBOX"}),
            json!({"subject": "Re: Angebot", "address": "rechnung@y.de", "name": null,
                   "received": 1_800_000_000, "mailbox": "imap://x/%5BGmail%5D/Gesendet"}),
        ];
        let hits = rank_rows(&rows, &rank::tokens("rechnung"), 1_800_000_000_000, 5);
        assert_eq!(hits[0].title, "Rechnung 1043");
        assert_eq!(
            hits[0].detail,
            "from Buchhaltung <buchhaltung@x.de> · INBOX"
        );
        assert_eq!(hits[1].detail, "from rechnung@y.de · Gesendet");
        assert_eq!(mailbox("imap://x/%5BGmail%5D"), "[Gmail]");
        assert_eq!(mailbox("imap://x/Entw%C3%BCrfe%"), "Entwürfe%");
    }
}
