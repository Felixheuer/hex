//! Running the small helpers a lookup needs (`mdfind`, `sqlite3`,
//! `osascript`) with a tight deadline and a cap on what they print, and
//! reading a database from a private copy so the app that owns it (Mail,
//! Chrome) is never locked or touched.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

/// `command`'s output, at most `max_bytes` of it (the helper is stopped
/// then), or why it failed: its error text, or that it took longer than
/// `timeout` (it is killed).
pub fn output(
    command: &mut Command,
    timeout: Duration,
    max_bytes: usize,
) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("couldn't start {:?}: {error}", command.get_program()))?;
    let mut stdout = child.stdout.take().ok_or("no output pipe")?;
    let mut stderr = child.stderr.take().ok_or("no error pipe")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        let mut full = false;
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    bytes.extend_from_slice(&chunk[..read]);
                    if bytes.len() >= max_bytes {
                        bytes.truncate(max_bytes);
                        full = true;
                        break;
                    }
                }
            }
        }
        let _ = sender.send((bytes, full));
    });
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.by_ref().take(8 * 1024).read_to_string(&mut text);
        text
    });
    let Ok((bytes, full)) = receiver.recv_timeout(timeout) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "{:?} took longer than {} ms",
            command.get_program(),
            timeout.as_millis()
        ));
    };
    if full {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    // Output ended: the helper is exiting; give it a moment, then stop it.
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    match status {
        Some(status) if status.success() => Ok(text),
        _ => {
            let error = errors.join().unwrap_or_default();
            let error = error.trim();
            Err(if error.is_empty() {
                format!("{:?} failed", command.get_program())
            } else {
                crate::assistant::store::shorten(error, 300)
            })
        }
    }
}

/// `sql`'s rows from a private copy of the SQLite database at `database`
/// (with its `-wal`/`-shm`/`-journal` beside it, so recent rows count).
/// On APFS the copy is a clone: milliseconds even for a large file.
pub fn query_copy(database: &Path, sql: &str, timeout: Duration) -> Result<Vec<Value>, String> {
    let dir = scratch_dir()?;
    let result = (|| {
        let name = database
            .file_name()
            .ok_or("not a database file")?
            .to_string_lossy()
            .into_owned();
        let copy = dir.join(&name);
        std::fs::copy(database, &copy).map_err(|error| copy_error(database, &error))?;
        for suffix in ["-wal", "-shm", "-journal"] {
            let side = PathBuf::from(format!("{}{suffix}", database.display()));
            if side.exists() {
                let _ = std::fs::copy(&side, dir.join(format!("{name}{suffix}")));
            }
        }
        let text = output(
            Command::new("/usr/bin/sqlite3")
                .args(["-readonly", "-json"])
                .arg(&copy)
                .arg(sql),
            timeout,
            4 * 1024 * 1024,
        )?;
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str::<Vec<Value>>(&text)
            .map_err(|error| format!("unreadable rows: {error}"))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn copy_error(database: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(1) {
        format!(
            "Hex may not read {} (it needs Full Disk Access in System Settings › Privacy & Security)",
            database.display()
        )
    } else {
        format!("couldn't read {}: {error}", database.display())
    }
}

/// A new folder private to this user for one copy.
fn scratch_dir() -> Result<PathBuf, String> {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hex-lookup-{}-{}-{}",
        std::process::id(),
        crate::assistant::store::now_ms(),
        COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    crate::app_paths::create_private_dir(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// `text` as an SQL string literal.
pub fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// `column LIKE '%token%'` with `%`, `_` and `\` in the token taken
/// literally.
pub fn like(column: &str, token: &str) -> String {
    let escaped: String = token
        .chars()
        .flat_map(|c| match c {
            '%' | '_' | '\\' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    format!(
        "{column} LIKE {} ESCAPE '\\'",
        literal(&format!("%{escaped}%"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_likes_are_escaped() {
        assert_eq!(literal("O'Brien"), "'O''Brien'");
        assert_eq!(
            like("s.subject", "50%_x"),
            r"s.subject LIKE '%50\%\_x%' ESCAPE '\'"
        );
        assert_eq!(like("a", "it's"), r"a LIKE '%it''s%' ESCAPE '\'");
    }

    #[test]
    fn a_helper_is_stopped_at_its_deadline_and_its_cap() {
        let slow = output(
            Command::new("/bin/sh").args(["-c", "sleep 5"]),
            Duration::from_millis(100),
            1024,
        );
        assert!(slow.unwrap_err().contains("took longer"));
        let capped = output(
            &mut Command::new("/usr/bin/yes"),
            Duration::from_secs(2),
            10,
        )
        .unwrap();
        assert_eq!(capped.len(), 10);
        let failed = output(
            Command::new("/bin/sh").args(["-c", "echo nope >&2; exit 3"]),
            Duration::from_secs(2),
            1024,
        );
        assert_eq!(failed.unwrap_err(), "nope");
    }

    #[test]
    fn a_database_is_read_from_a_copy() {
        let dir = scratch_dir().unwrap();
        let db = dir.join("t.db");
        let made = Command::new("/usr/bin/sqlite3")
            .arg(&db)
            .arg("CREATE TABLE t(a); INSERT INTO t VALUES ('x');")
            .status()
            .unwrap();
        assert!(made.success());
        let rows = query_copy(&db, "SELECT a FROM t;", Duration::from_secs(2)).unwrap();
        assert_eq!(rows, [serde_json::json!({"a": "x"})]);
        let missing = query_copy(&dir.join("none.db"), "SELECT 1;", Duration::from_secs(2));
        assert!(missing.unwrap_err().contains("couldn't read"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
