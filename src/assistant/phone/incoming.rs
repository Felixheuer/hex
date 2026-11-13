//! The person's replies, read from the Mac's Messages database
//! (`~/Library/Messages/chat.db`, read-only; macOS lets Hex read it only
//! with Full Disk Access). Only messages from the configured handle, and
//! only rows after a given one, are read.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use color_eyre::eyre::{Result, bail, eyre};
use serde_json::Value;

/// Seconds from the Unix epoch to Apple's (2001-01-01).
const APPLE_EPOCH: u64 = 978_307_200;

/// One reply.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub rowid: i64,
    /// When it was sent.
    pub sent: SystemTime,
    pub text: String,
}

impl Reply {
    /// Whether it was sent after `since` (older replies are never acted on).
    pub fn sent_after(&self, since: SystemTime) -> bool {
        self.sent >= since
    }
}

/// The Messages database.
pub fn database() -> Option<PathBuf> {
    crate::app_paths::home_dir().map(|home| home.join("Library/Messages/chat.db"))
}

/// Whether Hex may read the Messages database (Full Disk Access).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    Granted,
    /// macOS refuses: Full Disk Access is off.
    Denied,
    /// No Messages database here (Messages never set up).
    Missing,
}

pub fn access() -> Access {
    let Some(path) = database() else {
        return Access::Missing;
    };
    access_of(&path)
}

fn access_of(path: &Path) -> Access {
    match std::fs::File::open(path) {
        Ok(_) => Access::Granted,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Access::Missing,
        Err(_) => Access::Denied,
    }
}

/// System Settings › Privacy & Security › Full Disk Access.
pub fn open_full_disk_access() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
            .spawn();
    }
}

/// `handle` as Messages stores it, if it can only be a phone number or an
/// e-mail address (spaces and dashes in numbers dropped); `None` otherwise,
/// so nothing else ever reaches a query.
pub fn clean_handle(handle: &str) -> Option<String> {
    let handle = handle.trim();
    if handle.contains('@') {
        let ok = handle.len() <= 254
            && handle
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
            && handle.matches('@').count() == 1;
        return ok.then(|| handle.to_lowercase());
    }
    let digits: String = handle
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '(' | ')' | '/'))
        .collect();
    let ok = (7..=16).contains(&digits.len())
        && digits
            .chars()
            .enumerate()
            .all(|(index, c)| c.is_ascii_digit() || (index == 0 && c == '+'));
    ok.then(|| international(&digits, &region()))
}

/// A national number („0162 …“, „0049 …“) in the international form
/// iMessage needs („+49162 …“): a national one stays undeliverable, and
/// Messages stores the sender's number that way too. `region` is this
/// Mac's country („DE“); a number already international, or from a region
/// not known here, stays as it is.
pub fn international(digits: &str, region: &str) -> String {
    if digits.starts_with('+') {
        return digits.to_owned();
    }
    if let Some(rest) = digits.strip_prefix("00") {
        return format!("+{rest}");
    }
    let code = match region.to_ascii_uppercase().as_str() {
        "DE" => "49",
        "AT" => "43",
        "CH" => "41",
        "GB" => "44",
        "FR" => "33",
        "IT" => "39",
        "ES" => "34",
        "NL" => "31",
        "BE" => "32",
        "PL" => "48",
        "DK" => "45",
        "SE" => "46",
        "NO" => "47",
        _ => return digits.to_owned(),
    };
    match digits.strip_prefix('0') {
        Some(rest) => format!("+{code}{rest}"),
        None => digits.to_owned(),
    }
}

/// This Mac's country, from its preferred language („de-DE“ → „DE“).
fn region() -> String {
    let locale = crate::assistant::speech::system_locale();
    locale
        .rsplit(['-', '_'])
        .next()
        .filter(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
        .map(str::to_uppercase)
        .unwrap_or_else(|| "DE".into())
}

/// The query for `handle`'s replies after row `after` (the handle is
/// already [`clean_handle`]d, so it is a safe literal).
pub fn query(handle: &str, after: i64) -> String {
    format!(
        "SELECT m.ROWID AS rowid, m.date AS date, m.text AS text, hex(m.attributedBody) AS body \
         FROM message m JOIN handle h ON h.ROWID = m.handle_id \
         WHERE m.is_from_me = 0 AND h.id = '{handle}' AND m.ROWID > {after} \
         ORDER BY m.ROWID LIMIT 20;"
    )
}

/// The newest message row (replies count from after it).
pub fn newest_rowid() -> Result<i64> {
    let path = database().ok_or_else(|| eyre!("no home folder"))?;
    let rows = run(&path, "SELECT IFNULL(MAX(ROWID), 0) AS rowid FROM message;")?;
    Ok(rows
        .first()
        .and_then(|row| row.get("rowid"))
        .and_then(Value::as_i64)
        .unwrap_or(0))
}

/// `handle`'s replies after row `after`, oldest first.
pub fn new_replies(handle: &str, after: i64) -> Result<Vec<Reply>> {
    let path = database().ok_or_else(|| eyre!("no home folder"))?;
    replies_in(&path, handle, after)
}

fn replies_in(path: &Path, handle: &str, after: i64) -> Result<Vec<Reply>> {
    let Some(handle) = clean_handle(handle) else {
        bail!("not a phone number or e-mail address");
    };
    Ok(parse(&run(path, &query(&handle, after))?))
}

/// Runs `sql` on `path` read-only, rows as JSON objects.
fn run(path: &Path, sql: &str) -> Result<Vec<Value>> {
    let output = std::process::Command::new("/usr/bin/sqlite3")
        .args(["-readonly", "-json"])
        .arg(path)
        .arg(sql)
        .stdin(std::process::Stdio::null())
        .output()?;
    if !output.status.success() {
        bail!("sqlite3 failed (Full Disk Access?)");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str::<Vec<Value>>(&stdout)?)
}

/// Rows as replies; rows without text are left out.
pub fn parse(rows: &[Value]) -> Vec<Reply> {
    rows.iter()
        .filter_map(|row| {
            let rowid = row.get("rowid")?.as_i64()?;
            let date = row.get("date").and_then(Value::as_i64).unwrap_or(0);
            let text = row
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|text| !text.trim().is_empty())
                .or_else(|| {
                    let body = row.get("body")?.as_str()?;
                    attributed_text(&hex_bytes(body)?)
                })?;
            Some(Reply {
                rowid,
                sent: apple_time(date),
                text,
            })
        })
        .collect()
}

/// Apple's message date (nanoseconds, or seconds in old databases, since
/// 2001) as a time.
fn apple_time(date: i64) -> SystemTime {
    let date = u64::try_from(date).unwrap_or(0);
    let seconds = if date > 10_000_000_000 {
        date / 1_000_000_000
    } else {
        date
    };
    UNIX_EPOCH + Duration::from_secs(APPLE_EPOCH + seconds)
}

fn hex_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The text in a message's `attributedBody` (an NSAttributedString in
/// Apple's typedstream: after the `NSString` class name, `+`, a length,
/// then UTF-8). Newer macOS stores replies only there.
pub fn attributed_text(body: &[u8]) -> Option<String> {
    let at = body.windows(8).position(|window| window == b"NSString")?;
    let rest = &body[at + 8..];
    let plus = rest.iter().position(|byte| *byte == b'+')?;
    let rest = &rest[plus + 1..];
    let (len, start) = match *rest.first()? {
        0x81 => (
            usize::from(u16::from_le_bytes([*rest.get(1)?, *rest.get(2)?])),
            3,
        ),
        0x82 => (
            u32::from_le_bytes([*rest.get(1)?, *rest.get(2)?, *rest.get(3)?, *rest.get(4)?])
                as usize,
            5,
        ),
        short => (usize::from(short), 1),
    };
    let bytes = rest.get(start..start + len)?;
    String::from_utf8(bytes.to_vec())
        .ok()
        .filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_national_number_becomes_international() {
        assert_eq!(international("01621575563", "DE"), "+491621575563");
        assert_eq!(international("00491621575563", "DE"), "+491621575563");
        assert_eq!(international("+491621575563", "US"), "+491621575563");
        assert_eq!(international("06641234567", "AT"), "+436641234567");
        assert_eq!(international("4155551234", "US"), "4155551234");
    }

    #[test]
    fn only_numbers_and_addresses_are_handles() {
        assert_eq!(
            clean_handle("+49 170 123-4567").as_deref(),
            Some("+491701234567")
        );
        assert_eq!(
            clean_handle(" Felix@Example.com ").as_deref(),
            Some("felix@example.com")
        );
        for bad in ["x' OR 1=1 --", "+49 170; DROP", "a@b@c", "12", "abc"] {
            assert_eq!(clean_handle(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_query_reads_only_the_handles_newer_incoming_rows() {
        let sql = query("+491701234567", 42);
        assert!(sql.contains("h.id = '+491701234567'"));
        assert!(sql.contains("m.ROWID > 42"));
        assert!(sql.contains("m.is_from_me = 0"));
    }

    #[test]
    fn replies_come_from_text_or_the_attributed_body() {
        // "Ja" in a minimal typedstream after the NSString class name.
        let mut body =
            b"\x04\x0bstreamtyped\x81\xe8\x03\x84\x01@\x84\x84\x84\x08NSString\x01\x94\x84\x01+"
                .to_vec();
        body.push(2);
        body.extend_from_slice("Ja".as_bytes());
        body.extend_from_slice(b"\x86\x84");
        let hex: String = body.iter().map(|byte| format!("{byte:02X}")).collect();
        let rows = vec![
            serde_json::json!({"rowid": 7, "date": 700_000_000_000_000_000_i64, "text": "Status", "body": null}),
            serde_json::json!({"rowid": 8, "date": 700_000_001_000_000_000_i64, "text": null, "body": hex}),
            serde_json::json!({"rowid": 9, "date": 1, "text": "", "body": null}),
        ];
        let replies = parse(&rows);
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0].text, "Status");
        assert_eq!(replies[1].text, "Ja");
        let since = UNIX_EPOCH + Duration::from_secs(APPLE_EPOCH + 700_000_000);
        assert!(replies[1].sent_after(since));
        assert!(!replies[0].sent_after(since + Duration::from_secs(10)));
    }

    #[test]
    fn a_missing_or_unreadable_database_says_so() {
        assert_eq!(
            access_of(Path::new("/nonexistent/chat.db")),
            Access::Missing
        );
    }

    /// A real SQLite file with Messages' tables: only the handle's incoming
    /// rows after the cursor come back.
    #[cfg(target_os = "macos")]
    #[test]
    fn replies_are_read_from_a_messages_database() {
        let dir = std::env::temp_dir().join(format!("hex-phone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("chat.db");
        let _ = std::fs::remove_file(&db);
        let setup = "CREATE TABLE handle (ROWID INTEGER PRIMARY KEY, id TEXT);\
            CREATE TABLE message (ROWID INTEGER PRIMARY KEY, handle_id INTEGER, is_from_me INTEGER, date INTEGER, text TEXT, attributedBody BLOB);\
            INSERT INTO handle VALUES (1, '+491701234567'), (2, '+4930999');\
            INSERT INTO message VALUES (1, 1, 0, 700000000000000000, 'old', NULL);\
            INSERT INTO message VALUES (2, 1, 1, 700000001000000000, 'mine', NULL);\
            INSERT INTO message VALUES (3, 2, 0, 700000002000000000, 'someone else', NULL);\
            INSERT INTO message VALUES (4, 1, 0, 700000003000000000, 'Ja, privat', NULL);";
        let made = std::process::Command::new("/usr/bin/sqlite3")
            .arg(&db)
            .arg(setup)
            .status()
            .unwrap();
        assert!(made.success());
        let replies = replies_in(&db, "+49 170 1234567", 1).unwrap();
        assert_eq!(
            replies
                .iter()
                .map(|reply| reply.text.as_str())
                .collect::<Vec<_>>(),
            ["Ja, privat"]
        );
        assert_eq!(replies[0].rowid, 4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
