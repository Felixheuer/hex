//! Files through Spotlight (`mdfind` in the home folder): by name, by
//! content when names find too little, and recently used ones; and the
//! installed apps by name.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::exec;
use super::rank::{self, Hit};

/// The most paths one Spotlight query reads.
const MAX_PATHS: usize = 300;
const TIMEOUT: Duration = Duration::from_millis(2500);
/// Folders whose files are rarely what the person means.
const NOISE: [&str; 9] = [
    "/Library/",
    "/node_modules/",
    "/.git/",
    "/target/",
    "/.cache/",
    "/vendor/",
    "/.Trash/",
    "/DerivedData/",
    "/site-packages/",
];

/// A Spotlight value matching words that start with `token`, its quotes,
/// backslashes, and wildcards taken literally.
fn prefix(token: &str) -> String {
    let escaped: String = token
        .chars()
        .filter(|c| !c.is_control())
        .flat_map(|c| match c {
            '"' | '\\' | '*' | '?' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    format!("\"{escaped}*\"cdw")
}

/// Every token at a word's start in the name (`"rechnung*"cdw`: case,
/// diacritics, and word-wise).
pub fn name_query(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| format!("kMDItemDisplayName == {}", prefix(token)))
        .collect::<Vec<_>>()
        .join(" && ")
}

/// Every token in the file's text.
pub fn content_query(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| format!("kMDItemTextContent == {}", prefix(token)))
        .collect::<Vec<_>>()
        .join(" && ")
}

/// Used in the last `days` days, folders and apps left out, and with every
/// token in the name when there are tokens.
pub fn recent_query(tokens: &[String], days: u32) -> String {
    let mut query = format!(
        "kMDItemLastUsedDate >= $time.today(-{days}) && kMDItemContentType != public.folder && kMDItemContentType != com.apple.application-bundle"
    );
    if !tokens.is_empty() {
        query.push_str(" && ");
        query.push_str(&name_query(tokens));
    }
    query
}

/// One `mdfind -attr` line: the path and its last-used and changed dates.
#[derive(Debug, PartialEq)]
pub struct Found {
    pub path: String,
    pub used_ms: Option<i64>,
    pub changed_ms: Option<i64>,
}

/// `mdfind -attr kMDItemLastUsedDate -attr kMDItemFSContentChangeDate`
/// output: "path   kMDItemLastUsedDate = … +0000   kMDItemFSContentChangeDate = …".
pub fn parse(output: &str) -> Vec<Found> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split("   kMDItem");
            let path = parts.next().unwrap_or_default().to_owned();
            let mut found = Found {
                path,
                used_ms: None,
                changed_ms: None,
            };
            for part in parts {
                if let Some((key, value)) = part.split_once(" = ") {
                    let date = rank::parse_utc(value);
                    match key.trim() {
                        "LastUsedDate" => found.used_ms = date,
                        "FSContentChangeDate" => found.changed_ms = date,
                        _ => {}
                    }
                }
            }
            found
        })
        .collect()
}

fn mdfind(query: &str) -> Result<Vec<Found>, String> {
    let home = crate::app_paths::home_dir().ok_or("no home folder")?;
    let text = exec::output(
        Command::new("/usr/bin/mdfind")
            .arg("-onlyin")
            .arg(&home)
            .args([
                "-attr",
                "kMDItemLastUsedDate",
                "-attr",
                "kMDItemFSContentChangeDate",
            ])
            .arg(query),
        TIMEOUT,
        MAX_PATHS * 400,
    )?;
    let mut found = parse(&text);
    found.truncate(MAX_PATHS);
    Ok(found)
}

/// Ranks found paths: the name's match counts most, then recent use, a
/// shallow path, and not being in a build or library folder.
pub fn rank(found: Vec<Found>, tokens: &[String], by_content: bool, now_ms: i64) -> Vec<Hit> {
    found
        .into_iter()
        .map(|found| {
            let path = Path::new(&found.path);
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| found.path.clone());
            let matched = if by_content {
                0.4
            } else {
                rank::score(&name, tokens).max(0.1)
            };
            let when = found.used_ms.or(found.changed_ms);
            let depth = found.path.matches('/').count() as f64;
            let noise = NOISE.iter().any(|folder| found.path.contains(folder));
            let score = (matched + rank::recency(when, now_ms, 0.5) - depth * 0.01)
                * if noise { 0.3 } else { 1.0 };
            Hit {
                title: name,
                detail: found.path.clone(),
                when_ms: when,
                score: score.max(0.001),
            }
        })
        .collect()
}

/// Files whose names (else contents) hold every word of the query.
pub fn files(tokens: &[String], limit: usize, now_ms: i64) -> Result<Vec<Hit>, String> {
    if tokens.is_empty() {
        return recent(tokens, limit);
    }
    let mut hits = rank(mdfind(&name_query(tokens))?, tokens, false, now_ms);
    if hits.len() < limit {
        let by_content = mdfind(&content_query(tokens)).unwrap_or_default();
        hits.extend(rank(by_content, tokens, true, now_ms));
    }
    Ok(rank::top(hits, limit))
}

/// Files used in the last two weeks, newest first (named like the query,
/// when there is one).
pub fn recent(tokens: &[String], limit: usize) -> Result<Vec<Hit>, String> {
    let mut found = mdfind(&recent_query(tokens, 14))?;
    found.sort_by_key(|found| std::cmp::Reverse(found.used_ms));
    let hits = found
        .into_iter()
        .filter(|found| !NOISE.iter().any(|folder| found.path.contains(folder)))
        .map(|found| {
            let name = Path::new(&found.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Hit {
                title: name,
                detail: found.path,
                when_ms: found.used_ms,
                score: 1.0,
            }
        })
        .take(limit)
        .collect();
    Ok(hits)
}

/// Installed apps whose names match.
pub fn apps(tokens: &[String], limit: usize) -> Vec<Hit> {
    let home = crate::app_paths::home_dir().unwrap_or_default();
    let folders = [
        Path::new("/Applications").to_path_buf(),
        Path::new("/Applications/Utilities").to_path_buf(),
        Path::new("/System/Applications").to_path_buf(),
        Path::new("/System/Applications/Utilities").to_path_buf(),
        home.join("Applications"),
    ];
    let hits = folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let app = name.strip_suffix(".app")?.to_owned();
            Some(Hit {
                score: rank::score(&app, tokens),
                title: app,
                detail: entry.path().display().to_string(),
                when_ms: None,
            })
        })
        .collect();
    rank::top(hits, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_match_word_starts_and_escape_spotlight_syntax() {
        let tokens = rank::tokens("Rechnung März");
        assert_eq!(
            name_query(&tokens),
            r#"kMDItemDisplayName == "rechnung*"cdw && kMDItemDisplayName == "marz*"cdw"#
        );
        assert_eq!(
            content_query(&["a\"b*".into()]),
            r#"kMDItemTextContent == "a\"b\**"cdw"#
        );
        let recent = recent_query(&[], 7);
        assert!(recent.starts_with("kMDItemLastUsedDate >= $time.today(-7)"));
        assert!(!recent.contains("DisplayName"));
        assert!(recent_query(&tokens, 7).ends_with(r#""marz*"cdw"#));
    }

    #[test]
    fn spotlight_lines_parse_with_spaces_in_paths() {
        let found = parse(
            "/Users/f/Downloads/Bild 1.png   kMDItemLastUsedDate = 2026-11-13 09:58:27 +0000   kMDItemFSContentChangeDate = (null)\n\n/x   kMDItemLastUsedDate = (null)   kMDItemFSContentChangeDate = 1970-01-02 00:00:00 +0000\n",
        );
        assert_eq!(
            found,
            [
                Found {
                    path: "/Users/f/Downloads/Bild 1.png".into(),
                    used_ms: Some(1_794_563_907_000),
                    changed_ms: None,
                },
                Found {
                    path: "/x".into(),
                    used_ms: None,
                    changed_ms: Some(86_400_000),
                },
            ]
        );
    }

    #[test]
    fn a_named_recent_file_outside_build_folders_ranks_first() {
        let now = 1_800_000_000_000;
        let found = |path: &str, used: Option<i64>| Found {
            path: path.into(),
            used_ms: used,
            changed_ms: None,
        };
        let hits = rank(
            vec![
                found("/Users/f/proj/target/debug/rechnung.txt", Some(now)),
                found("/Users/f/Documents/Rechnungen alt.pdf", None),
                found(
                    "/Users/f/Documents/Rechnung Mai.pdf",
                    Some(now - 86_400_000),
                ),
            ],
            &rank::tokens("rechnung"),
            false,
            now,
        );
        let ranked = rank::top(hits, 3);
        assert_eq!(ranked[0].title, "Rechnung Mai.pdf");
        assert_eq!(ranked[2].title, "rechnung.txt");
    }
}
