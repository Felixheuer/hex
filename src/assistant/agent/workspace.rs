//! A job's own folder, `~/Documents/Hex/<date> <title>/`: its spreadsheets,
//! documents, intermediate data, its `plan.md` (the checklist Hex shows as
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

/// `~/Documents/Hex`.
pub fn root() -> Option<PathBuf> {
    Some(dirs::document_dir()?.join("Hex"))
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

/// `title` as a folder name: no path separators or leading dots, at most
/// 60 characters.
fn clean(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if matches!(c, '/' | ':' | '\\') || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = crate::assistant::store::shorten(cleaned.trim_start_matches('.'), 60);
    let cleaned = cleaned.trim_end_matches('…').trim();
    if cleaned.is_empty() {
        "Job".into()
    } else {
        cleaned.to_owned()
    }
}

/// The local date of `now_ms`, `YYYY-MM-DD`.
#[cfg(windows)]
fn date(now_ms: u64) -> String {
    let local = crate::assistant::schedules::local(now_ms);
    format!("{:04}-{:02}-{:02}", local.year, local.month, local.day)
}

/// The local date of `now_ms`, `YYYY-MM-DD`.
#[cfg(not(windows))]
fn date(now_ms: u64) -> String {
    let seconds = now_ms / 1000;
    #[cfg(unix)]
    {
        let seconds = seconds as libc::time_t;
        // SAFETY: a zeroed `tm` is valid; `localtime_r` writes only into it.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: both pointers are valid for the call.
        if !unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
            return format!(
                "{:04}-{:02}-{:02}",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday
            );
        }
    }
    let days = (seconds / 86_400) as i64;
    format!("day {days}")
}

/// The plan in `workspace/plan.md`: its `- [ ]` and `- [x]` lines.
pub fn plan(workspace: &Path) -> Vec<PlanStep> {
    fs::read_to_string(workspace.join(PLAN))
        .map(|text| parse_plan(&text))
        .unwrap_or_default()
}

/// The checklist lines of `text`.
pub fn parse_plan(text: &str) -> Vec<PlanStep> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start().strip_prefix(['-', '*'])?.trim_start();
            let (done, title) = if let Some(rest) = line.strip_prefix("[ ]") {
                (false, rest)
            } else {
                let rest = line
                    .strip_prefix("[x]")
                    .or_else(|| line.strip_prefix("[X]"))?;
                (true, rest)
            };
            let title = title.trim();
            (!title.is_empty()).then(|| PlanStep {
                title: crate::assistant::store::shorten(title, 120),
                done,
            })
        })
        .collect()
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
