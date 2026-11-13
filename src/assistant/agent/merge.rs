//! A team's results in one place: once every helper ended, Agent-Desk
//! combines what they wrote in their folders inside the lead's, so the
//! lead's last step starts from one table and one report instead of N.
//!
//! - **Tables**: every CSV a helper wrote (`rows.csv`, `suppliers.csv`, …)
//!   becomes one table, columns united by name in the order first seen,
//!   rows without duplicates (same web address, email, or name; else the
//!   same whole row). Written as `team-table.csv` and, through the media
//!   tools' writer, `team-table.xlsx`.
//! - **Reports**: each helper's `report.md` (else its final answer) under
//!   its title, as `team-report.md` and `team-report.docx`.
//!
//! The lead's `hex_team_status` runs it when no helper is left working and
//! names the files; the lead checks them and delivers (an online sheet, a
//! message). Its own files are never touched.
//!
//! OWNER: brain.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde_json::json;

use super::team::Member;

/// The combined table's and report's names in the lead's folder.
pub const TABLE: &str = "team-table";
pub const REPORT: &str = "team-report";
/// Rows a combined table keeps at most.
const MAX_ROWS: usize = 50_000;
/// Columns that name a row: a row with the same value in the first of them
/// both rows have is the same row.
const KEY_COLUMNS: &[&str] = &[
    "url",
    "website",
    "web",
    "webseite",
    "domain",
    "link",
    "email",
    "e-mail",
    "mail",
    "name",
    "firma",
    "company",
    "unternehmen",
    "title",
    "titel",
];

/// What was merged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Merged {
    /// The combined table (CSV), its rows, and the files it came from.
    pub table: Option<PathBuf>,
    pub rows: usize,
    pub duplicates: usize,
    pub sources: usize,
    /// The spreadsheet beside it, when the writer made one.
    pub sheet: Option<PathBuf>,
    /// The combined report (Markdown), and the document beside it.
    pub report: Option<PathBuf>,
    pub document: Option<PathBuf>,
}

impl Merged {
    /// What the lead is told.
    pub fn words(&self) -> Option<String> {
        let mut lines = Vec::new();
        if let Some(table) = &self.table {
            let sheet = self
                .sheet
                .as_ref()
                .map(|sheet| format!(" and {}", sheet.display()))
                .unwrap_or_default();
            lines.push(format!(
                "- One table: {}{sheet} ({} rows from {} file(s), {} duplicate(s) left out).",
                table.display(),
                self.rows,
                self.sources,
                self.duplicates
            ));
        }
        if let Some(report) = &self.report {
            let document = self
                .document
                .as_ref()
                .map(|document| format!(" and {}", document.display()))
                .unwrap_or_default();
            lines.push(format!("- One report: {}{document}.", report.display()));
        }
        (!lines.is_empty()).then(|| {
            format!(
                "\nAgent-Desk combined your helpers' results:\n{}\nCheck them, fix what's off, and deliver from these (import the table into the online sheet, or send the files).",
                lines.join("\n")
            )
        })
    }
}

/// The helpers' folders inside `lead`: its subfolders (a helper's
/// workspace), newest names last.
fn helper_folders(lead: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(lead)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .collect();
    folders.sort();
    folders
}

/// Every CSV in `folder` and one level below, in name order.
fn tables_in(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut look = |dir: &Path, deeper: &mut Vec<PathBuf>| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                deeper.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
            {
                found.push(path);
            }
        }
    };
    let mut below = Vec::new();
    look(folder, &mut below);
    for dir in below {
        look(&dir, &mut Vec::new());
    }
    found.sort();
    found
}

/// A header cell as compared: lowercase, trimmed.
fn column(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The CSV `tables` as one: united columns, rows without duplicates.
pub fn combine(tables: &[Vec<Vec<String>>]) -> (Vec<String>, Vec<Vec<String>>, usize) {
    let mut header: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for table in tables {
        for name in table.first().into_iter().flatten() {
            let key = column(name);
            if key.is_empty() || index.contains_key(&key) {
                continue;
            }
            index.insert(key, header.len());
            header.push(name.trim().to_owned());
        }
    }
    let keys: Vec<usize> = KEY_COLUMNS
        .iter()
        .filter_map(|name| index.get(*name).copied())
        .collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut rows = Vec::new();
    let mut duplicates = 0;
    for table in tables {
        let Some((first, body)) = table.split_first() else {
            continue;
        };
        let places: Vec<Option<usize>> = first
            .iter()
            .map(|name| index.get(&column(name)).copied())
            .collect();
        for cells in body {
            if cells.iter().all(|cell| cell.trim().is_empty()) {
                continue;
            }
            let mut row = vec![String::new(); header.len()];
            for (cell, place) in cells.iter().zip(&places) {
                if let Some(place) = place {
                    row[*place] = cell.trim().to_owned();
                }
            }
            let identity = keys
                .iter()
                .map(|&key| super::blackboard::key(&row[key]))
                .find(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    row.iter()
                        .map(|cell| cell.to_lowercase())
                        .collect::<Vec<_>>()
                        .join("\u{1f}")
                });
            if !seen.insert(identity) {
                duplicates += 1;
                continue;
            }
            if rows.len() < MAX_ROWS {
                rows.push(row);
            }
        }
    }
    (header, rows, duplicates)
}

fn csv_line(cells: &[String]) -> String {
    cells
        .iter()
        .map(|cell| {
            if cell.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", cell.replace('"', "\"\""))
            } else {
                cell.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Combines the results of `members` (helpers of the job whose folder is
/// `lead`) into the lead's folder; an empty [`Merged`] when none of them
/// wrote anything to combine.
pub fn merge(lead: &Path, members: &[Member]) -> Merged {
    let mut merged = Merged::default();
    let folders = helper_folders(lead);
    // Tables.
    let files: Vec<PathBuf> = folders
        .iter()
        .flat_map(|folder| tables_in(folder))
        .collect();
    let tables: Vec<Vec<Vec<String>>> = files
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|text| crate::assistant::media::document::read::csv(&text))
        .filter(|rows| rows.len() > 1)
        .collect();
    if !tables.is_empty() {
        let (header, rows, duplicates) = combine(&tables);
        let mut text = csv_line(&header);
        text.push('\n');
        for row in &rows {
            text.push_str(&csv_line(row));
            text.push('\n');
        }
        let path = lead.join(format!("{TABLE}.csv"));
        if std::fs::write(&path, text).is_ok() {
            merged.table = Some(path);
            merged.rows = rows.len();
            merged.duplicates = duplicates;
            merged.sources = tables.len();
            let mut sheet_rows = vec![header];
            sheet_rows.extend(rows);
            let sheet = lead.join(format!("{TABLE}.xlsx"));
            let written = crate::assistant::media::document::call_write(
                &json!({"path": sheet.display().to_string(), "overwrite": true,
                        "sheets": [{"name": "Team", "rows": sheet_rows}]}),
                lead,
            );
            if written.is_ok() && sheet.exists() {
                merged.sheet = Some(sheet);
            }
        }
    }
    // Reports.
    let mut sections = Vec::new();
    for member in members {
        let folder = folders.iter().find(|folder| {
            folder
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(member.title.trim()))
        });
        let report = folder
            .map(|folder| folder.join(super::workspace::REPORT))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .filter(|text| !text.trim().is_empty())
            .or_else(|| member.answer.clone().filter(|text| !text.trim().is_empty()));
        if let Some(report) = report {
            // A helper's own headings sit one level under its title.
            let body: String = report
                .trim()
                .lines()
                .map(|line| {
                    if line.starts_with('#') {
                        format!("#{line}\n")
                    } else {
                        format!("{line}\n")
                    }
                })
                .collect();
            sections.push(format!("## {}\n\n{}", member.title.trim(), body.trim_end()));
        }
    }
    if !sections.is_empty() {
        let markdown = format!("# Team report\n\n{}\n", sections.join("\n\n"));
        let path = lead.join(format!("{REPORT}.md"));
        if std::fs::write(&path, &markdown).is_ok() {
            merged.report = Some(path);
            let document = lead.join(format!("{REPORT}.docx"));
            let written = crate::assistant::media::document::call_write(
                &json!({"path": document.display().to_string(), "overwrite": true,
                        "title": "Team report", "markdown": markdown}),
                lead,
            );
            if written.is_ok() && document.exists() {
                merged.document = Some(document);
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::agent::team::State;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hex-merge-{name}-{}-{}",
            std::process::id(),
            crate::assistant::store::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn member(title: &str, answer: &str) -> Member {
        Member {
            id: format!("id-{title}"),
            title: title.into(),
            task: String::new(),
            state: State::Done,
            step: None,
            answer: Some(answer.into()),
            started_ms: 0,
        }
    }

    #[test]
    fn tables_unite_their_columns_and_drop_duplicates() {
        let a = vec![
            vec!["Name".to_owned(), "Website".into(), "Price".into()],
            vec!["Nike".into(), "https://www.nike.com/".into(), "10".into()],
            vec!["Puma".into(), "puma.com".into(), "8".into()],
        ];
        let b = vec![
            vec!["website".to_owned(), "name".into(), "Country".into()],
            vec!["nike.com".into(), "Nike Inc".into(), "US".into()],
            vec!["adidas.de".into(), "Adidas".into(), "DE".into()],
            vec![String::new(), String::new(), String::new()],
        ];
        let (header, rows, duplicates) = combine(&[a, b]);
        assert_eq!(header, ["Name", "Website", "Price", "Country"]);
        assert_eq!(duplicates, 1, "nike.com twice");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2], ["Adidas", "adidas.de", "", "DE"]);
    }

    #[test]
    fn helpers_results_become_one_table_and_one_report() {
        let lead = folder("lead");
        std::fs::write(lead.join("own.csv"), "x\n1\n").unwrap();
        let nike = lead.join("2026-11-13 Nike");
        let puma = lead.join("2026-11-13 Puma");
        std::fs::create_dir_all(nike.join("data")).unwrap();
        std::fs::create_dir_all(&puma).unwrap();
        std::fs::write(
            nike.join("data/rows.csv"),
            "Name,Website\nNike,nike.com\n\"Nike, Outlet\",nikeoutlet.com\n",
        )
        .unwrap();
        std::fs::write(
            puma.join("rows.csv"),
            "Name;Website\nPuma;puma.com\nNike;https://nike.com\n",
        )
        .unwrap();
        std::fs::write(puma.join("report.md"), "# Puma\nFound 1 shop.\n").unwrap();
        let merged = merge(
            &lead,
            &[member("Nike", "Nike: 2 shops."), member("Puma", "x")],
        );
        assert_eq!((merged.rows, merged.duplicates, merged.sources), (3, 1, 2));
        let table = std::fs::read_to_string(merged.table.as_ref().unwrap()).unwrap();
        assert!(table.starts_with("Name,Website\n"), "{table}");
        assert!(table.contains("\"Nike, Outlet\",nikeoutlet.com"), "{table}");
        assert!(
            !table.contains("x\n1"),
            "the lead's own files stay out: {table}"
        );
        let report = std::fs::read_to_string(merged.report.as_ref().unwrap()).unwrap();
        assert!(report.contains("## Nike\n\nNike: 2 shops."), "{report}");
        assert!(
            report.contains("## Puma\n\n## Puma\nFound 1 shop."),
            "{report}"
        );
        let words = merged.words().unwrap();
        assert!(
            words.contains("3 rows from 2 file(s), 1 duplicate(s)"),
            "{words}"
        );
        assert_eq!(merge(&folder("empty"), &[]).words(), None);
        let _ = std::fs::remove_dir_all(&lead);
    }
}
