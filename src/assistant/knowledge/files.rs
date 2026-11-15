//! Files through Spotlight (`mdfind` in the home folder): by name, by
//! content when names find too little, and recently used ones. (Installed
//! apps are the guard's `hex_installed_apps`.)

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
    /// When it came into its folder (a download, a file saved there).
    pub added_ms: Option<i64>,
}

impl Found {
    /// Its newest date: used, changed, or added.
    fn newest_ms(&self) -> Option<i64> {
        [self.used_ms, self.changed_ms, self.added_ms]
            .into_iter()
            .flatten()
            .max()
    }
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
                added_ms: None,
            };
            for part in parts {
                if let Some((key, value)) = part.split_once(" = ") {
                    let date = rank::parse_utc(value);
                    match key.trim() {
                        "LastUsedDate" => found.used_ms = date,
                        "FSContentChangeDate" => found.changed_ms = date,
                        "DateAdded" => found.added_ms = date,
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
    mdfind_in(&home, query)
}

fn mdfind_in(folder: &Path, query: &str) -> Result<Vec<Found>, String> {
    let text = exec::output(
        Command::new("/usr/bin/mdfind")
            .arg("-onlyin")
            .arg(folder)
            .args([
                "-attr",
                "kMDItemLastUsedDate",
                "-attr",
                "kMDItemFSContentChangeDate",
                "-attr",
                "kMDItemDateAdded",
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

/// Where and what the person means besides a file's name: a folder they
/// named ("in Downloads", "auf dem Schreibtisch"), a kind ("pdf"), and the
/// words left for the name.
#[derive(Debug, PartialEq)]
pub struct Wanted {
    /// A folder in the home folder ("Downloads").
    pub folder: Option<&'static str>,
    /// A file extension, lowercase ("pdf").
    pub kind: Option<String>,
    pub words: Vec<String>,
}

/// Folded words naming a folder in the home folder.
const FOLDERS: [(&str, &str); 10] = [
    ("downloads", "Downloads"),
    ("download", "Downloads"),
    ("downloadordner", "Downloads"),
    ("schreibtisch", "Desktop"),
    ("desktop", "Desktop"),
    ("dokumente", "Documents"),
    ("documents", "Documents"),
    ("dokumenten", "Documents"),
    ("bilder", "Pictures"),
    ("pictures", "Pictures"),
];

/// Extensions a request names as a kind of file.
const KINDS: [&str; 22] = [
    "pdf", "docx", "doc", "xlsx", "xls", "csv", "pptx", "ppt", "key", "pages", "numbers", "png",
    "jpg", "jpeg", "heic", "zip", "txt", "md", "mp3", "mp4", "mov", "dmg",
];

/// Words that only glue a folder or kind into a sentence.
const GLUE: [&str; 20] = [
    "in", "im", "auf", "dem", "der", "die", "das", "den", "ordner", "folder", "datei", "dateien",
    "file", "files", "the", "on", "my", "meine", "mein", "meinem",
];

/// Splits `tokens` into a folder, a kind, and the words for the name; the
/// glue words go only when a folder or kind was named.
pub fn wanted(tokens: &[String]) -> Wanted {
    let mut folder = None;
    let mut kind = None;
    let mut words = Vec::new();
    for token in tokens {
        let bare = token.trim_start_matches('.');
        if let Some((_, name)) = FOLDERS.iter().find(|(word, _)| *word == token) {
            folder.get_or_insert(*name);
        } else if kind.is_none() && KINDS.contains(&bare) {
            kind = Some(bare.to_owned());
        } else {
            words.push(token.clone());
        }
    }
    if folder.is_some() || kind.is_some() {
        words.retain(|word| !GLUE.contains(&word.as_str()));
    }
    Wanted {
        folder,
        kind,
        words,
    }
}

/// Files in `kind` (any when `None`), named with every word when there are
/// words, else changed, used, or added in the last `days` days.
pub fn scoped_query(wanted: &Wanted, days: u32) -> String {
    let mut parts = vec!["kMDItemContentType != public.folder".to_owned()];
    if let Some(kind) = &wanted.kind {
        parts.push(format!("kMDItemFSName == \"*.{kind}\"c"));
    }
    if wanted.words.is_empty() {
        parts.push(format!(
            "(kMDItemDateAdded >= $time.today(-{days}) || kMDItemLastUsedDate >= $time.today(-{days}) || kMDItemFSContentChangeDate >= $time.today(-{days}))"
        ));
    } else {
        parts.push(name_query(&wanted.words));
    }
    parts.join(" && ")
}

/// Files in the folder or of the kind the query names: by name, else the
/// newest there.
fn scoped(wanted: &Wanted, limit: usize, now_ms: i64) -> Result<Vec<Hit>, String> {
    let home = crate::app_paths::home_dir().ok_or("no home folder")?;
    let folder = match wanted.folder {
        Some(name) if home.join(name).is_dir() => home.join(name),
        _ => home,
    };
    let found = mdfind_in(&folder, &scoped_query(wanted, 60))?;
    if !wanted.words.is_empty() {
        return Ok(rank::top(rank(found, &wanted.words, false, now_ms), limit));
    }
    Ok(newest(found, limit))
}

/// `found` newest first (used, changed, or added), build folders left out.
fn newest(mut found: Vec<Found>, limit: usize) -> Vec<Hit> {
    found.retain(|found| !NOISE.iter().any(|folder| found.path.contains(folder)));
    found.sort_by_key(|found| std::cmp::Reverse(found.newest_ms()));
    found
        .into_iter()
        .take(limit)
        .map(|found| Hit {
            title: Path::new(&found.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            when_ms: found.newest_ms(),
            detail: found.path,
            score: 1.0,
        })
        .collect()
}

/// Files whose names (else contents) hold every word of the query. A
/// folder ("Downloads") or kind ("pdf") in the query narrows the search
/// first; with nothing found there, every word counts for the name again.
pub fn files(tokens: &[String], limit: usize, now_ms: i64) -> Result<Vec<Hit>, String> {
    if tokens.is_empty() {
        return recent(tokens, limit);
    }
    let wanted = wanted(tokens);
    if wanted.folder.is_some() || wanted.kind.is_some() {
        match scoped(&wanted, limit, now_ms) {
            Ok(hits) if !hits.is_empty() => return Ok(hits),
            Ok(_) => {}
            Err(error) => tracing::debug!(%error, "scoped file lookup failed"),
        }
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
    let wanted = wanted(tokens);
    if wanted.folder.is_some() || wanted.kind.is_some() {
        let home = crate::app_paths::home_dir().ok_or("no home folder")?;
        let folder = wanted
            .folder
            .map(|name| home.join(name))
            .filter(|folder| folder.is_dir())
            .unwrap_or(home);
        let found = mdfind_in(&folder, &scoped_query(&wanted, 14))?;
        if !found.is_empty() {
            return Ok(newest(found, limit));
        }
    }
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
                    added_ms: None,
                },
                Found {
                    path: "/x".into(),
                    used_ms: None,
                    changed_ms: Some(86_400_000),
                    added_ms: None,
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
            added_ms: None,
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

    #[test]
    fn a_named_folder_and_kind_narrow_the_search() {
        let wanted = wanted(&rank::tokens("die PDF in Downloads"));
        assert_eq!(
            wanted,
            Wanted {
                folder: Some("Downloads"),
                kind: Some("pdf".into()),
                words: vec![],
            }
        );
        let query = scoped_query(&wanted, 7);
        assert!(query.contains(r#"kMDItemFSName == "*.pdf"c"#));
        assert!(query.contains("kMDItemDateAdded >= $time.today(-7)"));
        let named = wanted_words("Rechnung Schreibtisch");
        assert_eq!(named.folder, Some("Desktop"));
        assert_eq!(named.words, ["rechnung"]);
        assert!(scoped_query(&named, 7).contains("kMDItemDisplayName"));
        // Without a folder or kind, glue words stay name words.
        assert_eq!(wanted_words("die Zeit").words, ["die", "zeit"]);
    }

    fn wanted_words(query: &str) -> Wanted {
        wanted(&rank::tokens(query))
    }

    #[test]
    fn the_newest_download_comes_first() {
        let found = |path: &str, added: i64| Found {
            path: path.into(),
            used_ms: None,
            changed_ms: Some(1),
            added_ms: Some(added),
        };
        let hits = newest(
            vec![
                found("/Users/f/Downloads/alt.pdf", 10),
                found("/Users/f/Downloads/neu.pdf", 99),
                found("/Users/f/Downloads/x/node_modules/a.pdf", 500),
            ],
            5,
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "neu.pdf");
        assert_eq!(hits[0].when_ms, Some(99));
    }

    #[test]
    fn spotlight_lines_read_the_added_date() {
        let found = parse(
            "/a.pdf   kMDItemLastUsedDate = (null)   kMDItemFSContentChangeDate = (null)   kMDItemDateAdded = 1970-01-02 00:00:00 +0000\n",
        );
        assert_eq!(found[0].added_ms, Some(86_400_000));
    }
}
