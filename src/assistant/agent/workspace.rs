//! A job's own folder, `~/Documents/Agent-Desk/<date> <title>/`: its spreadsheets,
//! documents, intermediate data, its `plan.md` (the checklist Agent-Desk shows as
//! the plan), and its `report.md` (everything it did, with links). The
//! backend's file and shell tools are scoped to it.

use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::{Result, eyre::eyre};

use crate::assistant::types::PlanStep;

/// The plan's file in a workspace.
pub const PLAN: &str = "plan.md";
/// The report's file in a workspace.
pub const REPORT: &str = "report.md";
/// What a job proves at its end: what was done, links, screenshots and
/// diff images, message proofs (the person reads it after coming back).
pub const EVIDENCE: &str = "Nachweis.md";

/// `~/Documents/Agent-Desk`.
pub fn root() -> Option<PathBuf> {
    Some(crate::app_paths::document_dir()?.join("Agent-Desk"))
}

/// Creates a new workspace for `title` under `root`, named
/// `<YYYY-MM-DD> <title>` (with " 2", " 3", … when taken).
pub fn create(root: &Path, title: &str, now_ms: u64) -> Result<PathBuf> {
    let name = format!("{} {}", date(now_ms), clean(title));
    fs::create_dir_all(root)
        .map_err(|error| eyre!("couldn't create {}: {error}", root.display()))?;
    for n in 1..1000 {
        let path = if n == 1 {
            root.join(&name)
        } else {
            root.join(format!("{name} {n}"))
        };
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(eyre!("couldn't create {}: {error}", path.display())),
        }
    }
    Err(eyre!("too many workspaces named “{name}”"))
}

/// `title` as a folder name on every system: no path separators, leading
/// dots, or characters Windows refuses (`< > " | ? *`), no trailing dots or
/// spaces, not a name Windows reserves (`CON`, `NUL`, `COM1`…), at most 60
/// characters.
pub(crate) fn clean(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if matches!(c, '/' | ':' | '\\' | '<' | '>' | '"' | '|' | '?' | '*') || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = crate::assistant::store::shorten(cleaned.trim_start_matches('.'), 60);
    let cleaned = cleaned
        .trim_end_matches('…')
        .trim()
        .trim_end_matches(['.', ' '])
        .trim();
    if cleaned.is_empty() {
        return "Job".into();
    }
    if reserved(cleaned) {
        return format!("{cleaned} job");
    }
    cleaned.to_owned()
}

/// A name Windows keeps for a device, with or without an extension.
fn reserved(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|device| {
            stem.strip_prefix(device).is_some_and(|digit| {
                matches!(
                    digit,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

/// The local date of `now_ms`, `YYYY-MM-DD`.
fn date(now_ms: u64) -> String {
    if let Some(tm) = crate::assistant::schedules::local_tm(now_ms) {
        return format!(
            "{:04}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        );
    }
    let days = (now_ms / 1000 / 86_400) as i64;
    format!("day {days}")
}

/// The plan in `workspace/plan.md`: its `- [ ]` and `- [x]` lines.
pub fn plan(workspace: &Path) -> Vec<PlanStep> {
    fs::read_to_string(workspace.join(PLAN))
        .map(|text| parse_plan(&text))
        .unwrap_or_default()
}

/// The checklist lines of `text`, as the card shows them (without their
/// success criteria).
pub fn parse_plan(text: &str) -> Vec<PlanStep> {
    parse_steps(text)
        .into_iter()
        .map(|step| PlanStep {
            title: crate::assistant::store::shorten(&step.title, 120),
            done: step.done,
        })
        .collect()
}

/// One step of a job's plan with its success criterion: `- [ ] Research 50
/// suppliers (browser) — check: `suppliers.xlsx` has 50 rows`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub title: String,
    pub done: bool,
    /// How the step proves it is done (what follows `check:`).
    pub check: Option<String>,
}

/// The steps in `workspace/plan.md`.
pub fn steps(workspace: &Path) -> Vec<Step> {
    fs::read_to_string(workspace.join(PLAN))
        .map(|text| parse_steps(&text))
        .unwrap_or_default()
}

/// The checklist lines of `text`, each with its criterion.
pub fn parse_steps(text: &str) -> Vec<Step> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start().strip_prefix(['-', '*'])?.trim_start();
            let (done, rest) = if let Some(rest) = line.strip_prefix("[ ]") {
                (false, rest)
            } else {
                let rest = line
                    .strip_prefix("[x]")
                    .or_else(|| line.strip_prefix("[X]"))?;
                (true, rest)
            };
            let (title, check) = split_check(rest.trim());
            (!title.is_empty()).then(|| Step {
                title: title.to_owned(),
                done,
                check,
            })
        })
        .collect()
}

/// A step's words split at its `check:`: the title before it (without the
/// dash or bracket that leads to it) and the criterion after it.
fn split_check(text: &str) -> (&str, Option<String>) {
    let Some(at) = ["check:", "Check:", "CHECK:"]
        .iter()
        .filter_map(|marker| text.find(marker))
        .min()
    else {
        return (text, None);
    };
    let title = text[..at]
        .trim_end()
        .trim_end_matches(['—', '–', '-', '|', '(', ';', ',']);
    let check = text[at + "check:".len()..]
        .trim()
        .trim_end_matches(')')
        .trim()
        .to_owned();
    (title.trim(), (!check.is_empty()).then_some(check))
}

/// What Agent-Desk can check of a step's criterion itself: every file it
/// names in backticks (`report.xlsx`, `out/data.csv`) must exist, in the
/// workspace or at its absolute path. `None` when it names no file;
/// otherwise the files that are missing (empty when all are there).
pub fn missing_files(workspace: &Path, check: &str) -> Option<Vec<String>> {
    let named: Vec<&str> = check
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|name| looks_like_file(name))
        .collect();
    if named.is_empty() {
        return None;
    }
    Some(
        named
            .into_iter()
            .filter(|name| {
                let path = Path::new(name);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    workspace.join(path)
                };
                !path.exists()
            })
            .map(str::to_owned)
            .collect(),
    )
}

/// A backticked name that is a file: no spaces or call syntax, with an
/// extension or a path separator.
fn looks_like_file(name: &str) -> bool {
    !name.is_empty()
        && !name.contains([' ', '(', '=', '<', '>'])
        && !name.contains("://")
        && (name.contains('/')
            || name
                .rsplit_once('.')
                .is_some_and(|(stem, ext)| !stem.is_empty() && (1..=5).contains(&ext.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_is_named_after_the_date_and_title_and_never_reused() {
        let root = std::env::temp_dir().join(format!(
            "hex-workspace-{}-{}",
            std::process::id(),
            crate::assistant::store::now_ms()
        ));
        let now = 1_794_472_200_000; // 2026-11-12, midday in Europe
        let first = create(&root, "Supplier research: blabla.com/shop", now).unwrap();
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.ends_with(" Supplier research blabla.com shop"),
            "{name}"
        );
        assert!(name.starts_with("2026-11-1"), "{name}");
        assert!(first.is_dir());
        let second = create(&root, "Supplier research: blabla.com/shop", now).unwrap();
        assert!(second.to_string_lossy().ends_with(" 2"));
        assert!(create(&root, "../..", now).unwrap().starts_with(&root));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_workspace_name_is_valid_on_windows_too() {
        assert_eq!(clean(r#"What? "Q3" <draft> | a*b"#), "What Q3 draft a b");
        assert_eq!(clean("Notes..."), "Notes");
        assert_eq!(clean("Plan. "), "Plan");
        assert_eq!(clean("con"), "con job");
        assert_eq!(clean("COM1"), "COM1 job");
        assert_eq!(clean("nul.txt"), "nul.txt job");
        assert_eq!(clean("Console"), "Console");
        assert_eq!(clean("???"), "Job");
    }

    #[test]
    fn each_step_carries_its_check_and_the_card_shows_only_the_step() {
        let text = "- [x] Find 50 suppliers (browser) — check: `suppliers.csv` has 50 rows\n- [ ] Build the sheet (Check: `out/sheet.xlsx` opens)\n- [ ] Draft emails\n";
        let steps = parse_steps(text);
        assert_eq!(steps[0].title, "Find 50 suppliers (browser)");
        assert_eq!(
            steps[0].check.as_deref(),
            Some("`suppliers.csv` has 50 rows")
        );
        assert_eq!(steps[1].title, "Build the sheet");
        assert_eq!(steps[1].check.as_deref(), Some("`out/sheet.xlsx` opens"));
        assert_eq!(steps[2].check, None);
        assert_eq!(parse_plan(text)[0].title, "Find 50 suppliers (browser)");
        let dir = std::env::temp_dir().join(format!(
            "hex-workspace-check-{}-{}",
            std::process::id(),
            crate::assistant::store::now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("suppliers.csv"), "a").unwrap();
        assert_eq!(
            missing_files(&dir, "`suppliers.csv` has 50 rows"),
            Some(vec![])
        );
        assert_eq!(
            missing_files(&dir, "`out/sheet.xlsx` opens, `suppliers.csv` too"),
            Some(vec!["out/sheet.xlsx".to_owned()])
        );
        assert_eq!(missing_files(&dir, "verify_state shows `Sent`"), None);
        assert_eq!(missing_files(&dir, "`npm test` passes"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_plan_is_the_checklist_in_plan_md() {
        let steps = parse_plan(
            "# Plan\n- [x] Look at blabla.com (browser)\n- [ ] Research 50 suppliers (5 parallel batches)\n  * [ ] Draft emails in Gmail\nNotes: none\n- plain item\n",
        );
        assert_eq!(
            steps,
            [
                PlanStep {
                    title: "Look at blabla.com (browser)".into(),
                    done: true
                },
                PlanStep {
                    title: "Research 50 suppliers (5 parallel batches)".into(),
                    done: false
                },
                PlanStep {
                    title: "Draft emails in Gmail".into(),
                    done: false
                },
            ]
        );
    }
}
