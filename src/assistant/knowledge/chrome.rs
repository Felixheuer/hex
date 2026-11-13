//! The person's own Google Chrome: its open tabs (one Apple Event round
//! per property through JavaScript for Automation, which never launches
//! Chrome) and its history (titles, URLs, last visits, from a copy of
//! `History`; never cookies, logins, or form data). Incognito windows are
//! left out.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::exec;
use super::rank::{self, Hit};

/// Microseconds between 1601-01-01 (Chrome's epoch) and 1970-01-01.
const CHROME_EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;

/// One open tab of the person's Chrome.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub window_id: i64,
    /// The window's place, front first (1 is the front window).
    pub window: usize,
    pub tab_id: i64,
    /// The tab's place in its window, from 1.
    pub index: usize,
    pub title: String,
    pub url: String,
    pub active: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Listing {
    running: bool,
    ids: Vec<i64>,
    modes: Vec<String>,
    active: Vec<usize>,
    tab_ids: Vec<Vec<i64>>,
    titles: Vec<Vec<String>>,
    urls: Vec<Vec<String>>,
}

const LIST_SCRIPT: &str = r#"function run() {
  var chrome = Application('Google Chrome');
  if (!chrome.running()) return JSON.stringify({running: false});
  var w = chrome.windows;
  return JSON.stringify({running: true, ids: w.id(), modes: w.mode(), active: w.activeTabIndex(),
    tabIds: w.tabs.id(), titles: w.tabs.title(), urls: w.tabs.url()});
}"#;

/// Runs a JavaScript-for-Automation script with `args`.
pub fn jxa(script: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    exec::output(
        Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", script])
            .args(args),
        timeout,
        2 * 1024 * 1024,
    )
    .map(|text| text.trim_end_matches('\n').to_owned())
    .map_err(|error| {
        if error.contains("-1743") || error.contains("Not authorized") {
            "Hex may not control Google Chrome yet (System Settings › Privacy & Security › Automation › Hex › Google Chrome)".into()
        } else if error.contains("-600") || error.contains("Application can't be found") {
            "Google Chrome isn't installed or didn't answer".into()
        } else {
            error
        }
    })
}

/// The listing as tabs, incognito windows left out.
fn parse_listing(text: &str) -> Result<Option<Vec<Tab>>, String> {
    let listing: Listing =
        serde_json::from_str(text).map_err(|error| format!("unreadable tab list: {error}"))?;
    if !listing.running {
        return Ok(None);
    }
    let mut tabs = Vec::new();
    for (window, id) in listing.ids.iter().enumerate() {
        if listing
            .modes
            .get(window)
            .is_some_and(|mode| mode == "incognito")
        {
            continue;
        }
        let active = listing.active.get(window).copied().unwrap_or(0);
        let titles = listing.titles.get(window).cloned().unwrap_or_default();
        let urls = listing.urls.get(window).cloned().unwrap_or_default();
        let ids = listing.tab_ids.get(window).cloned().unwrap_or_default();
        for (index, tab_id) in ids.iter().enumerate() {
            tabs.push(Tab {
                window_id: *id,
                window: window + 1,
                tab_id: *tab_id,
                index: index + 1,
                title: titles.get(index).cloned().unwrap_or_default(),
                url: urls.get(index).cloned().unwrap_or_default(),
                active: index + 1 == active,
            });
        }
    }
    Ok(Some(tabs))
}

/// The person's open Chrome tabs; `None` when Chrome isn't running.
pub fn open_tabs() -> Result<Option<Vec<Tab>>, String> {
    parse_listing(&jxa(LIST_SCRIPT, &[], Duration::from_secs(3))?)
}

/// `url` without a query that carries a secret (a token, key, code, or
/// session in its parameters).
pub fn safe_url(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let lower = query.to_lowercase();
    let secret = [
        "token", "key", "code", "secret", "password", "passwd", "session", "auth", "sig",
    ]
    .iter()
    .any(|word| lower.contains(word));
    if secret {
        format!("{base}?…")
    } else {
        url.to_owned()
    }
}

/// How a tab reads in a result.
pub fn tab_hit(tab: &Tab, score: f64) -> Hit {
    Hit {
        title: if tab.title.is_empty() {
            tab.url.clone()
        } else {
            tab.title.clone()
        },
        detail: format!(
            "{} · window {}, tab {}{} · tab id {}",
            safe_url(&tab.url),
            tab.window,
            tab.index,
            if tab.active { " (showing)" } else { "" },
            tab.tab_id
        ),
        when_ms: None,
        score,
    }
}

/// Tabs matching the query; the showing tab of the front window first
/// among equals. No query: every tab, front window first.
pub fn rank_tabs(tabs: &[Tab], tokens: &[String], limit: usize) -> Vec<Hit> {
    let hits = tabs
        .iter()
        .map(|tab| {
            let matched = rank::score(&format!("{} {}", tab.title, tab.url), tokens);
            let bonus = if tab.active { 0.1 } else { 0.0 } + 0.05 / tab.window as f64;
            tab_hit(tab, if matched > 0.0 { matched + bonus } else { 0.0 })
        })
        .collect();
    rank::top(hits, limit)
}

pub fn tabs(tokens: &[String], limit: usize) -> Result<Vec<Hit>, String> {
    match open_tabs()? {
        None => Err("Google Chrome isn't running".into()),
        Some(tabs) => Ok(rank_tabs(&tabs, tokens, limit)),
    }
}

/// Chrome's profiles that have a history, the last used first.
fn histories() -> Vec<PathBuf> {
    let Some(home) = crate::app_paths::home_dir() else {
        return Vec::new();
    };
    let root = home.join("Library/Application Support/Google/Chrome");
    let last_used = std::fs::read(root.join("Local State"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|state| state["profile"]["last_used"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "Default".into());
    let mut profiles: Vec<String> = std::fs::read_dir(&root)
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
        .filter(|path| path.exists())
        .take(4)
        .collect()
}

/// Every word in the title or URL; the latest 300 visits.
pub fn history_sql(words: &[String]) -> String {
    let mut sql = String::from(
        "SELECT title, url, last_visit_time AS visited, visit_count AS visits FROM urls WHERE hidden = 0",
    );
    for word in words {
        sql.push_str(&format!(
            " AND ({} OR {})",
            exec::like("title", word),
            exec::like("url", word)
        ));
    }
    sql.push_str(" ORDER BY last_visit_time DESC LIMIT 300;");
    sql
}

/// Chrome's time (µs since 1601) as Unix ms.
pub fn chrome_time_ms(micros: i64) -> Option<i64> {
    (micros > CHROME_EPOCH_OFFSET_US).then(|| (micros - CHROME_EPOCH_OFFSET_US) / 1000)
}

pub fn rank_history(rows: &[Value], tokens: &[String], now_ms: i64, limit: usize) -> Vec<Hit> {
    let hits = rows
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
                .and_then(chrome_time_ms);
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
                detail: safe_url(&url),
                when_ms: when,
                score,
            })
        })
        .collect();
    rank::top(hits, limit)
}

pub fn history(query: &str, limit: usize, now_ms: i64) -> Result<Vec<Hit>, String> {
    let histories = histories();
    if histories.is_empty() {
        return Err("no Chrome history found".into());
    }
    let sql = history_sql(&rank::words(query));
    let mut rows = Vec::new();
    let mut error = None;
    for history in histories {
        match exec::query_copy(&history, &sql, Duration::from_secs(3)) {
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
    fn the_tab_listing_parses_without_incognito_windows() {
        let text = json!({"running": true, "ids": [11, 12, 13], "modes": ["normal", "incognito", "normal"],
            "active": [2, 1, 1],
            "tabIds": [[101, 102], [201], [301]],
            "titles": [["Gmail", "Kalibermarkt – Staging"], ["Secret"], ["GitHub"]],
            "urls": [["https://mail.google.com/", "https://staging.kalibermarkt.de/"], ["https://x"], ["https://github.com/"]]})
        .to_string();
        let tabs = parse_listing(&text).unwrap().unwrap();
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[1].tab_id, 102);
        assert!(tabs[1].active && !tabs[0].active);
        assert_eq!(tabs[2].window, 3);
        assert!(tabs.iter().all(|tab| tab.title != "Secret"));
        assert_eq!(parse_listing(r#"{"running":false}"#).unwrap(), None);

        let hits = rank_tabs(&tabs, &rank::tokens("kalibermarkt"), 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].detail,
            "https://staging.kalibermarkt.de/ · window 1, tab 2 (showing) · tab id 102"
        );
        assert_eq!(rank_tabs(&tabs, &[], 5)[0].title, "Kalibermarkt – Staging");
    }

    #[test]
    fn history_reads_titles_urls_and_visits_only() {
        let sql = history_sql(&rank::words("github rust"));
        assert!(sql.starts_with("SELECT title, url, last_visit_time"));
        assert_eq!(sql.matches("title LIKE").count(), 2);
        assert!(!sql.to_lowercase().contains("cookie"));
        assert_eq!(
            chrome_time_ms(13_438_969_176_982_841),
            Some(1_794_495_576_982)
        );
        assert_eq!(chrome_time_ms(0), None);
        let now = 1_794_495_576_982;
        let rows = [
            json!({"title": "GitHub", "url": "https://github.com/", "visited": 13_438_969_176_982_841_i64, "visits": 50}),
            json!({"title": "Trending Rust", "url": "https://github.com/trending/rust?since=daily", "visited": 13_438_949_090_101_952_i64, "visits": 1}),
            json!({"title": "Reset", "url": "https://x.de/reset?token=abc", "visited": 13_438_949_090_101_952_i64, "visits": 1}),
        ];
        let hits = rank_history(&rows, &rank::tokens("github"), now, 5);
        assert_eq!(hits[0].title, "GitHub");
        assert_eq!(hits.len(), 2);
        let reset = rank_history(&rows, &rank::tokens("reset"), now, 5);
        assert_eq!(reset[0].detail, "https://x.de/reset?…");
    }
}
