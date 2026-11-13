use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

const FILE_FAIL: usize = 1500;
const FOLDER_FAIL: usize = 60;
const SIZE_EXC: &str = "scripts/file-size-exceptions.txt";
const FOLDER_EXC: &str = "scripts/folder-shape-exceptions.txt";

const SKIPPED_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    "vendored",
    "third_party",
    "target",
    "dist",
    "build",
    ".build",
    "out",
    "generated",
    "__generated__",
    "gen",
    "fixture",
    "fixtures",
    "Pods",
    "DerivedData",
];

const TEST_DIRS: &[&str] = &["test", "tests", "benches", "e2e", "__tests__"];

fn is_source_ext(path: &str) -> bool {
    path.ends_with(".rs")
        || path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with(".swift")
}

fn is_skipped(path: &str) -> bool {
    if path.ends_with(".d.ts") || path.ends_with("_generated.rs") {
        return true;
    }
    for infix in [".generated.", ".gen.", ".g.", ".pb."] {
        for ext in ["rs", "ts", "tsx", "swift"] {
            if path.ends_with(&format!("{infix}{ext}")) {
                return true;
            }
        }
    }
    path.split('/')
        .rev()
        .skip(1)
        .any(|seg| SKIPPED_DIRS.contains(&seg))
}

fn is_source(path: &str) -> bool {
    is_source_ext(path) && !is_skipped(path)
}

fn is_test(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if matches!(name, "test.rs" | "tests.rs")
        || name.ends_with("_test.rs")
        || name.ends_with("_tests.rs")
        || name.ends_with(".test.ts")
        || name.ends_with(".test.tsx")
        || name.ends_with(".spec.ts")
        || name.ends_with(".spec.tsx")
        || name.ends_with("Tests.swift")
    {
        return true;
    }
    path.split('/')
        .rev()
        .skip(1)
        .any(|seg| TEST_DIRS.contains(&seg) || seg.ends_with("Tests"))
}

fn braces(line: &str) -> isize {
    let mut no_escapes = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let _ = chars.next();
        } else {
            no_escapes.push(c);
        }
    }

    let mut no_strings = String::with_capacity(no_escapes.len());
    let mut rest = no_escapes.as_str();
    while let Some(open) = rest.find('"') {
        no_strings.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('"') {
            rest = &after[close + 1..];
        } else {
            no_strings.push_str(&rest[open..]);
            rest = "";
            break;
        }
    }
    no_strings.push_str(rest);

    let chars: Vec<char> = no_strings.chars().collect();
    let mut no_chars = String::with_capacity(no_strings.len());
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i] == '\'' && chars[i + 2] == '\'' {
            i += 3;
        } else {
            no_chars.push(chars[i]);
            i += 1;
        }
    }

    let cleaned = match no_chars.find("//") {
        Some(idx) => &no_chars[..idx],
        None => no_chars.as_str(),
    };
    let opens = cleaned.bytes().filter(|&b| b == b'{').count() as isize;
    let closes = cleaned.bytes().filter(|&b| b == b'}').count() as isize;
    opens - closes
}

fn measure_code_lines(path: &str, text: &str) -> usize {
    if is_test(path) {
        return 0;
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if !path.ends_with(".rs") {
        return lines.len();
    }

    let mut code = 0;
    let mut state = 0_u8;
    let mut depth = 0_isize;
    for line in lines {
        let trimmed = line.trim_start();
        if state == 0 {
            if trimmed.starts_with("#[cfg(test)]") {
                state = 1;
            } else {
                code += 1;
            }
            continue;
        }
        if state == 1 {
            if trimmed.starts_with("#[") {
                continue;
            }
            let d = braces(line);
            if d > 0 || line.contains('{') {
                depth = d;
                state = if depth > 0 { 2 } else { 0 };
            } else if line.trim_end().ends_with(';') {
                state = 0;
            }
            continue;
        }
        depth += braces(line);
        if depth <= 0 {
            state = 0;
        }
    }
    code
}

fn parse_exceptions(text: &str, strip_wildcard: bool) -> HashMap<String, Option<usize>> {
    let mut out = HashMap::new();
    for raw in text.split('\n') {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(raw_path) = parts.next() else {
            continue;
        };
        let mut path = raw_path.trim_end_matches('/').to_owned();
        if strip_wildcard && let Some(stripped) = path.strip_suffix('*') {
            path = stripped
                .strip_suffix('_')
                .or_else(|| stripped.strip_suffix('-'))
                .unwrap_or(stripped)
                .to_owned();
        }
        let ceiling = parts
            .next()
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<usize>().ok());
        out.insert(path, ceiling);
    }
    out
}

fn dir_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[..idx],
        None => ".",
    }
}

fn check_hard_violations(
    files: &[String],
    code_lines: &HashMap<String, usize>,
    size_exc_text: &str,
    folder_exc_text: &str,
) -> Vec<String> {
    let sources: Vec<&str> = files
        .iter()
        .map(String::as_str)
        .filter(|p| is_source(p))
        .collect();
    let code: Vec<&str> = sources.iter().copied().filter(|p| !is_test(p)).collect();
    let size_exc = parse_exceptions(size_exc_text, false);
    let folder_exc = parse_exceptions(folder_exc_text, true);

    let allowed = |exc: &HashMap<String, Option<usize>>, key: &str, count: usize| -> bool {
        match exc.get(key) {
            None => false,
            Some(None) => true,
            Some(Some(ceiling)) => count <= *ceiling,
        }
    };

    let mut failures = Vec::new();
    for &path in &sources {
        let Some(&lines) = code_lines.get(path) else {
            continue;
        };
        if lines > FILE_FAIL && !allowed(&size_exc, path, lines) {
            let limit = size_exc.get(path).and_then(|c| *c).unwrap_or(FILE_FAIL);
            failures.push(format!(
                "file over limit: {path} has {lines} non-test lines (limit {limit})"
            ));
        }
    }

    let mut dirs = BTreeSet::new();
    for path in files {
        let parts: Vec<&str> = path.split('/').collect();
        for i in 1..parts.len() {
            dirs.insert(parts[..i].join("/"));
        }
    }

    let mut by_folder: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for &path in &code {
        by_folder.entry(dir_of(path)).or_default().push(path);
    }

    for (folder, paths) in by_folder {
        if paths.len() > FOLDER_FAIL && !allowed(&folder_exc, folder, paths.len()) {
            let limit = folder_exc
                .get(folder)
                .and_then(|c| *c)
                .unwrap_or(FOLDER_FAIL);
            failures.push(format!(
                "folder over limit: {folder} has {} non-test source files (limit {limit})",
                paths.len()
            ));
        }

        let last_seg = folder.rsplit('/').next().unwrap_or(folder);
        let is_program_dir = matches!(last_seg, "examples" | "benches" | "bin");
        let mut stems: Vec<(&str, &str)> = Vec::new();
        for &path in &paths {
            let name = path.rsplit('/').next().unwrap_or(path);
            if let Some(stem) = name.strip_suffix(".rs") {
                if !is_program_dir {
                    stems.push(("rs", stem));
                }
            } else if let Some(stem) = name
                .strip_suffix(".tsx")
                .or_else(|| name.strip_suffix(".ts"))
            {
                stems.push(("ts", stem));
            }
        }

        let mut members: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for &(lang, stem) in &stems {
            let split_at = if lang == "rs" {
                stem.find('_')
            } else {
                stem.find(['-', '_'])
            };
            if let Some(at) = split_at
                && at > 0
                && at + 1 < stem.len()
            {
                *members.entry((lang, &stem[..at])).or_insert(0) += 1;
            }
        }

        for ((lang, prefix), count) in members {
            let base = if folder == "." {
                prefix.to_owned()
            } else {
                format!("{folder}/{prefix}")
            };
            let anchored =
                dirs.contains(&base) || stems.iter().any(|&(l, s)| l == lang && s == prefix);
            let threshold = if anchored { 2 } else { 3 };
            if count >= threshold && !allowed(&folder_exc, &base, count) {
                let sep = if lang == "rs" { '_' } else { '-' };
                failures.push(format!(
                    "prefix family: {base}{sep}* has {count} files (move into {base}/)"
                ));
            }
        }
    }

    failures
}

#[test]
fn repo_satisfies_code_shape() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join(".git").exists() {
        return;
    }
    let Ok(output) = crate::desktop::background_process::command("git")
        .args(["-C", env!("CARGO_MANIFEST_DIR"), "ls-files", "-z"])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let files: Vec<String> = raw
        .split('\0')
        .filter(|p| {
            !p.is_empty() && !p.starts_with("node_modules/") && !p.contains("/node_modules/")
        })
        .map(str::to_owned)
        .collect();

    let mut code_lines = HashMap::new();
    for path in &files {
        if is_source(path)
            && let Ok(bytes) = fs::read(root.join(path))
        {
            let text = String::from_utf8_lossy(&bytes);
            code_lines.insert(path.clone(), measure_code_lines(path, &text));
        }
    }

    let size_exc = fs::read_to_string(root.join(SIZE_EXC)).unwrap_or_default();
    let folder_exc = fs::read_to_string(root.join(FOLDER_EXC)).unwrap_or_default();
    let failures = check_hard_violations(&files, &code_lines, &size_exc, &folder_exc);
    assert!(
        failures.is_empty(),
        "code-shape check failed (run `python3 scripts/code-shape.py --check`):\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn inline_cfg_test_items_are_excluded_from_code_lines() {
    let src = "\
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod external_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds() {
        assert_eq!(add(1, 2), 3);
    }
}
";
    assert_eq!(measure_code_lines("src/math.rs", src), 5);
    assert_eq!(measure_code_lines("src/math_tests.rs", src), 0);
}

#[test]
fn prefix_families_and_size_ceilings_are_enforced() {
    let files = vec![
        "src/foo.rs".to_owned(),
        "src/foo_one.rs".to_owned(),
        "src/foo_two.rs".to_owned(),
        "src/big.rs".to_owned(),
    ];
    let mut lines = HashMap::new();
    lines.insert("src/foo.rs".to_owned(), 20);
    lines.insert("src/foo_one.rs".to_owned(), 20);
    lines.insert("src/foo_two.rs".to_owned(), 20);
    lines.insert("src/big.rs".to_owned(), 1600);

    let failures = check_hard_violations(&files, &lines, "", "");
    assert_eq!(failures.len(), 2, "{failures:?}");

    let allowed = check_hard_violations(&files, &lines, "src/big.rs 1600\n", "src/foo_* 2\n");
    assert!(allowed.is_empty(), "{allowed:?}");

    let exceeded = check_hard_violations(&files, &lines, "src/big.rs 1599\n", "src/foo_* 1\n");
    assert_eq!(exceeded.len(), 2, "{exceeded:?}");
}

/// Every place that opens or raises Agent-Desk's window, by file and the
/// function the call is in: explicit asks only (docs/ux/redesign.md §5).
/// Everything the assistant shows by itself goes to the overlays (the
/// stage, the bubble, the repair card). A new caller fails this test until
/// it's added here with why the person asked for the window.
const WINDOW_OPENERS: &[(&str, &str, &str)] = &[
    // The openers themselves.
    ("src/rune_ui/window_link.rs", "open_chat", "the opener"),
    (
        "src/rune_ui/window_link.rs",
        "open_assistant_setup",
        "the opener",
    ),
    ("src/rune_ui/window_link.rs", "open_repair", "the opener"),
    ("src/macos_rune.rs", "open_chat", "the opener"),
    ("src/macos_rune.rs", "open_assistant_setup", "the opener"),
    ("src/macos_rune.rs", "open_repair", "the opener"),
    ("src/macos_rune.rs", "bring_forward", "the opener"),
    ("src/macos_rune.rs", "show", "the opener"),
    ("src/macos_rune.rs", "show_update_check", "the opener"),
    ("src/macos_rune.rs", "show_settings", "the opener"),
    // The assistant's explicit asks.
    (
        "src/rune_ui/window_link.rs",
        "wire_assistant",
        "the stage's Open in Agent-Desk (behind ⋯), Continue setup, the repair card's Fix all…",
    ),
    (
        "src/rune_ui/window_link.rs",
        "notification_clicked",
        "a notification's own Open Agent-Desk action (its click goes to the stage)",
    ),
    // The menu bar, the Dock, the CLI, links: the person asked.
    ("src/macos_rune.rs", "status_action", "menu bar items"),
    (
        "src/macos_rune.rs",
        "handle_reopen",
        "a Dock click or reopening the app",
    ),
    (
        "src/macos_rune.rs",
        "start_main_thread",
        "no menu bar item could be made: the window is the only way in",
    ),
    (
        "src/macos_rune.rs",
        "developer",
        "`agent-desk dev` pane commands",
    ),
    (
        "src/macos_rune/control.rs",
        "show_window",
        "`agent-desk window show`",
    ),
    (
        "src/macos_rune/control.rs",
        "show_story",
        "`agent-desk window story`",
    ),
    (
        "src/macos_rune/control.rs",
        "answer",
        "`agent-desk onboarding reset`",
    ),
    (
        "src/rune_ui.rs",
        "on_open_url",
        "an agent-desk:// link the person opened",
    ),
    (
        "src/windows/rune.rs",
        "create_tray",
        "tray items, or no tray",
    ),
    (
        "src/windows/rune.rs",
        "show_window",
        "`agent-desk window show`",
    ),
    (
        "src/windows/rune.rs",
        "onboarding_reset",
        "`agent-desk onboarding reset`",
    ),
    // Meetings (not the assistant): a capsule's Open, and failures the
    // window explains. Outside this pass; they stay as they are.
    (
        "src/macos_rune.rs",
        "controller_event",
        "meetings: Open on a capsule, a failed recording",
    ),
    (
        "src/meeting/runtime.rs",
        "follow",
        "meetings: Open on a capsule, a failed recording",
    ),
    (
        "src/meeting/runtime.rs",
        "offer_unshown",
        "meetings: an offer that couldn't show",
    ),
];

/// The callers of the window's openers in `text` (a source file), as
/// (function, call): `open_chat(`, `bring_window_forward(`, `window.open()`,
/// `show(&self.window`, and their kin; definitions, comments, and test
/// modules aside.
fn window_openers(text: &str) -> Vec<(String, String)> {
    const CALLS: &[&str] = &[
        "open_chat(",
        "open_assistant_setup(",
        "open_repair(",
        "bring_window_forward(",
        "bring_forward(",
        "show_update_check(",
        "show(&self.window",
        "show(&pump.window",
        "show(window,",
        "window.open()",
    ];
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut within: Option<String> = None;
    for (at, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[cfg(test)]")
            && lines
                .get(at + 1)
                .is_some_and(|next| next.trim().starts_with("mod ") && next.trim().ends_with('{'))
        {
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        if let Some((before, rest)) = line.split_once("fn ")
            && (before.is_empty() || before.ends_with(' ') || before.ends_with('('))
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                within = Some(name);
            }
        }
        for call in CALLS {
            let mut from = 0;
            while let Some(found_at) = line[from..].find(call) {
                let start = from + found_at;
                from = start + call.len();
                let before = line[..start].chars().next_back();
                let part_of_a_name = before.is_some_and(|c| c.is_alphanumeric() || c == '_');
                let defined = line[..start].ends_with("fn ");
                if part_of_a_name || defined {
                    continue;
                }
                found.push((within.clone().unwrap_or_default(), (*call).to_owned()));
            }
        }
    }
    found
}

#[test]
fn only_explicit_entry_points_open_the_window() {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    let allowed: BTreeSet<(&str, &str)> = WINDOW_OPENERS
        .iter()
        .map(|(file, function, _)| (*file, *function))
        .collect();
    let mut seen = BTreeSet::new();
    let mut unexpected = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if is_test(&relative) || relative == "src/code_shape_tests.rs" {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for (function, call) in window_openers(&text) {
            if allowed.contains(&(relative.as_str(), function.as_str())) {
                seen.insert((relative.clone(), function));
            } else {
                unexpected.push(format!("{relative}: {function} calls {call}"));
            }
        }
    }
    assert!(
        unexpected.is_empty(),
        "the window opens only when the person asks; these callers aren't on the allow-list \
         (WINDOW_OPENERS): send what they show to the overlays, or add them with why:\n{}",
        unexpected.join("\n")
    );
    let stale: Vec<_> = allowed
        .iter()
        .filter(|(file, function)| !seen.contains(&((*file).to_owned(), (*function).to_owned())))
        .collect();
    assert!(
        stale.is_empty(),
        "allow-list entries no longer used: {stale:?}"
    );
}

#[test]
fn the_opener_scan_finds_calls_and_skips_definitions_and_comments() {
    let source = "fn open_chat(conversation: Option<String>) {\n    window.open();\n}\n\
        fn later() {\n    // open_chat(None) in a comment\n    crate::rune_ui::window_link::open_chat(None);\n    reopen_chat(x);\n}\n\
        #[cfg(test)]\nmod tests {\n    fn t() { open_chat(None); }\n}\n";
    assert_eq!(
        window_openers(source),
        [
            ("open_chat".to_owned(), "window.open()".to_owned()),
            ("later".to_owned(), "open_chat(".to_owned()),
        ]
    );
}
