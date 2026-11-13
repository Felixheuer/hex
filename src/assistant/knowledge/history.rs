//! The person's browsing history (titles, URLs, last visits; never
//! cookies, logins, or form data), read from copies of the browsers' own
//! databases: every Chromium-based browser's profiles (a folder under
//! Application Support with a `Local State` and `<profile>/History`, the
//! same layout whatever its name) and Safari's `History.db`. No list of
//! browsers: the layout is what's looked for.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::exec;
use super::rank::{self, Hit};

/// Most history databases read for one lookup.
const MAX_DATABASES: usize = 6;

/// A history database and how it is asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Database {
    Chromium(PathBuf),
    Safari(PathBuf),
}

impl Database {
    fn path(&self) -> &Path {
        match self {
            Self::Chromium(path) | Self::Safari(path) => path,
        }
    }

    /// The rows matching every word, newest first, with `visited` in Unix ms.
    pub fn sql(&self, words: &[String]) -> String {
        let (title, url) = match self {
            Self::Chromium(_) => ("title", "url"),
            Self::Safari(_) => ("v.title", "i.url"),
        };
        let mut filter = String::new();
        for word in words {
            filter.push_str(&format!(
                " AND ({} OR {})",
                exec::like(title, word),
                exec::like(url, word)
            ));
        }
        match self {
            // µs since 1601.
            Self::Chromium(_) => format!(
                "SELECT title, url, (last_visit_time - 11644473600000000) / 1000 AS visited, \
                 visit_count AS visits FROM urls WHERE hidden = 0{filter} \
                 ORDER BY last_visit_time DESC LIMIT 300;"
            ),
            // Seconds since 2001.
            Self::Safari(_) => format!(
                "SELECT v.title AS title, i.url AS url, \
                 CAST((MAX(v.visit_time) + 978307200) * 1000 AS INTEGER) AS visited, \
                 i.visit_count AS visits FROM history_items i \
                 JOIN history_visits v ON v.history_item = i.id WHERE 1 = 1{filter} \
                 GROUP BY i.id ORDER BY visited DESC LIMIT 300;"
            ),
        }
    }
}

/// The Chromium user-data folders under `support` (Application Support):
/// one or two levels down (`Vendor/Browser`), each with a `Local State`.
pub fn chromium_roots(support: &Path) -> Vec<PathBuf> {
    let children = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut roots = Vec::new();
    for first in children(support) {
        if first.join("Local State").is_file() {
            roots.push(first);
            continue;
        }
        for second in children(&first) {
            if second.join("Local State").is_file() {
                roots.push(second);
            }
        }
    }
    roots.sort();
    roots
}

/// A Chromium user-data folder's histories, the last used profile first.
pub fn chromium_histories(root: &Path) -> Vec<PathBuf> {
    let last_used = std::fs::read(root.join("Local State"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|state| state["profile"]["last_used"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "Default".into());
    let mut profiles: Vec<String> = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name == "Default" || name.starts_with("Profile "))
                .collect()
        })
        .unwrap_or_default();
    profiles.sort_by_key(|name| (name != &last_used, name.clone()));
    profiles
        .into_iter()
        .map(|profile| root.join(profile).join("History"))
        .filter(|path| path.is_file())
        .collect()
}

/// Every history database on this Mac, the most recently used first.
fn databases() -> Vec<Database> {
    let Some(home) = crate::app_paths::home_dir() else {
        return Vec::new();
    };
    let mut found: Vec<(Option<std::time::SystemTime>, Database)> = Vec::new();
    for root in chromium_roots(&home.join("Library/Application Support")) {
        for path in chromium_histories(&root) {
            found.push((modified(&path), Database::Chromium(path)));
        }
    }
    let safari = home.join("Library/Safari/History.db");
    if safari.exists() {
        found.push((modified(&safari), Database::Safari(safari)));
    }
    found.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
    found
        .into_iter()
        .map(|(_, database)| database)
        .take(MAX_DATABASES)
        .collect()
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    [path, wal.as_path()]
        .iter()
        .filter_map(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        })
        .max()
}

pub fn rank_history(rows: &[Value], tokens: &[String], now_ms: i64, limit: usize) -> Vec<Hit> {
    let mut hits: Vec<Hit> = rows
        .iter()
        .filter_map(|row| {
            let url = row.get("url")?.as_str()?.to_owned();
            let title = row
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let when = row
                .get("visited")
                .and_then(Value::as_i64)
                .filter(|ms| *ms > 0);
            let visits = row
                .get("visits")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                .max(1);
            let matched = rank::score(&format!("{title} {url}"), tokens);
            let score = if matched > 0.0 {
                matched + rank::recency(when, now_ms, 0.4) + (visits as f64).ln() * 0.03
            } else {
                0.0
            };
            Some(Hit {
                title: if title.is_empty() { url.clone() } else { title },
                detail: super::browser::safe_url(&url),
                when_ms: when,
                score,
            })
        })
        .collect();
    // The same page in several browsers or profiles: once.
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert(hit.detail.clone()));
    rank::top(hits, limit)
}

pub fn history(query: &str, limit: usize, now_ms: i64) -> Result<Vec<Hit>, String> {
    let databases = databases();
    if databases.is_empty() {
        return Err("no browser history found".into());
    }
    let words = rank::words(query);
    let mut rows = Vec::new();
    let mut error = None;
    for database in databases {
        match exec::query_copy(
            database.path(),
            &database.sql(&words),
            Duration::from_secs(3),
        ) {
            Ok(found) => rows.extend(found),
            Err(why) => error = Some(why),
        }
    }
    if rows.is_empty()
        && let Some(error) = error
    {
        return Err(error);
    }
    Ok(rank_history(&rows, &rank::tokens(query), now_ms, limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn history_reads_titles_urls_and_visits_only() {
        let words = rank::words("github rust");
        let chromium = Database::Chromium(PathBuf::from("History")).sql(&words);
        assert!(chromium.starts_with("SELECT title, url, (last_visit_time"));
        assert_eq!(chromium.matches("title LIKE").count(), 2);
        let safari = Database::Safari(PathBuf::from("History.db")).sql(&words);
        assert!(safari.contains("FROM history_items i"));
        assert_eq!(safari.matches("v.title LIKE").count(), 2);
        for sql in [&chromium, &safari] {
            assert!(!sql.to_lowercase().contains("cookie"));
        }
        let now = 1_794_495_576_982;
        let rows = [
            json!({"title": "GitHub", "url": "https://github.com/", "visited": now, "visits": 50}),
            json!({"title": "GitHub", "url": "https://github.com/", "visited": now - 9, "visits": 3}),
            json!({"title": "Trending Rust", "url": "https://github.com/trending/rust?since=daily", "visited": now - 20_000_000, "visits": 1}),
            json!({"title": "Reset", "url": "https://x.de/reset?token=abc", "visited": now - 20_000_000, "visits": 1}),
        ];
        let hits = rank_history(&rows, &rank::tokens("github"), now, 5);
        assert_eq!(hits[0].title, "GitHub");
        assert_eq!(hits.len(), 2, "the same page once");
        let reset = rank_history(&rows, &rank::tokens("reset"), now, 5);
        assert_eq!(reset[0].detail, "https://x.de/reset?…");
    }

    #[test]
    fn chromium_profiles_are_found_by_their_layout() {
        let support = std::env::temp_dir().join(format!(
            "hex-history-test-{}-{}",
            std::process::id(),
            crate::assistant::store::now_ms()
        ));
        let make = |path: &Path| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        };
        make(&support.join("Vendor/Browser/Local State"));
        make(&support.join("Vendor/Browser/Default/History"));
        make(&support.join("Vendor/Browser/Profile 2/History"));
        make(&support.join("Other/Local State"));
        make(&support.join("Other/Default/History"));
        make(&support.join("Editor/settings.json"));
        let roots = chromium_roots(&support);
        assert_eq!(
            roots,
            [support.join("Other"), support.join("Vendor/Browser")]
        );
        std::fs::write(
            support.join("Vendor/Browser/Local State"),
            br#"{"profile": {"last_used": "Profile 2"}}"#,
        )
        .unwrap();
        assert_eq!(
            chromium_histories(&support.join("Vendor/Browser")),
            [
                support.join("Vendor/Browser/Profile 2/History"),
                support.join("Vendor/Browser/Default/History")
            ]
        );
        let _ = std::fs::remove_dir_all(&support);
    }
}
