//! What an app action really reaches, for the guard's checks: every app a
//! script addresses (not only the one it is wrapped in), and whether it
//! runs code beyond the apps' own dictionaries (a shell, another script,
//! a raw Apple event, the Objective-C bridge, an executable file). Outside
//! YOLO such an action is refused, since text a task read on a page could
//! have written it; the checks are the same for every app.

use std::path::Path;

use serde_json::Value;

/// File extensions that run something when opened (or point at a file
/// that does).
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "app",
    "command",
    "tool",
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "fish",
    "py",
    "rb",
    "pl",
    "php",
    "js",
    "scpt",
    "scptd",
    "applescript",
    "workflow",
    "action",
    "terminal",
    "shortcut",
    "pkg",
    "mpkg",
    "jar",
    "jnlp",
    "prefpane",
    "osax",
    "webloc",
    "fileloc",
    "inetloc",
    "url",
];

/// Phrases of a script (lowercase, whitespace collapsed) that run code
/// beyond an app's dictionary, and what they do.
const RUNS_CODE: &[(&str, &str)] = &[
    ("do shell script", "runs shell commands"),
    ("doshellscript", "runs shell commands"),
    ("do script", "runs commands in a terminal"),
    ("doscript", "runs commands in a terminal"),
    ("run script", "runs another script"),
    ("runscript", "runs another script"),
    ("load script", "runs another script"),
    ("«event", "sends a raw Apple event"),
    ("current application's", "uses the Objective-C bridge"),
    ("use framework", "uses the Objective-C bridge"),
    ("call method", "uses the Objective-C bridge"),
    ("objc.", "uses the Objective-C bridge"),
    ("$.ns", "uses the Objective-C bridge"),
    ("do javascript", "runs JavaScript in a browser tab"),
    ("dojavascript", "runs JavaScript in a browser tab"),
    ("open for access", "writes files directly"),
    ("keystroke", "types keys into whatever is in front"),
    ("key code", "types keys into whatever is in front"),
    ("open location", "opens a URL (use intent open_url)"),
];

/// Words that may follow `application` without naming an app.
const NOT_A_NAME: &[&str] = &[
    "process",
    "processes",
    "file",
    "files",
    "support",
    "to",
    "of",
    "in",
    "is",
    "and",
    "or",
    "then",
    "else",
    "end",
    "with",
    "from",
    "at",
    "by",
    "as",
    "whose",
    "where",
    "name",
    "names",
    "version",
    "class",
    "the",
];

/// Every app `script` addresses by a literal (`application "Mail"`,
/// `app "Mail"`, `application id "com.apple.mail"`, a path, raw
/// `«class capp»`, JXA's `Application('Mail')`), and whether it addresses
/// one by a computed name (`tell application x`, `application ("Ma" &
/// "il")`), which can't be checked.
pub fn targets(script: &str) -> (Vec<String>, bool) {
    static CAPP: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let capp = CAPP.get_or_init(|| {
        regex::Regex::new(r"(?i)(?:«|<<)\s*class\s+capp\s*(?:»|>>)").expect("valid pattern")
    });
    let text = capp.replace_all(script, " application ").into_owned();
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut found = Vec::new();
    let mut dynamic = false;
    let mut at = 0;
    while at < bytes.len() {
        let rest = &lower[at..];
        let keyword = if rest.starts_with("application") {
            "application".len()
        } else if rest.starts_with("app") {
            "app".len()
        } else {
            at += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let start = at;
        at += keyword;
        if (start > 0 && word(bytes[start - 1])) || bytes.get(at).copied().is_some_and(word) {
            continue;
        }
        if lower[..start].trim_end().ends_with("current") {
            continue;
        }
        let mut cursor = at;
        let skip = |cursor: &mut usize| {
            while bytes
                .get(*cursor)
                .is_some_and(|c| *c == b' ' || *c == b'\t')
            {
                *cursor += 1;
            }
        };
        skip(&mut cursor);
        if lower[cursor..].starts_with("id")
            && bytes
                .get(cursor + 2)
                .is_some_and(|c| c.is_ascii_whitespace() || *c == b'"' || *c == b'(')
        {
            cursor += 2;
            skip(&mut cursor);
        }
        let literal = |cursor: &mut usize, quote: u8| -> Option<String> {
            let open = *cursor + 1;
            let close = lower[open..].find(quote as char)? + open;
            *cursor = close + 1;
            Some(text[open..close].to_owned())
        };
        match bytes.get(cursor).copied() {
            Some(b'"') => match literal(&mut cursor, b'"') {
                Some(name) => {
                    skip(&mut cursor);
                    if bytes.get(cursor) == Some(&b'&') {
                        dynamic = true;
                    }
                    found.push(name);
                }
                None => dynamic = true,
            },
            Some(b'(') => {
                cursor += 1;
                skip(&mut cursor);
                match bytes.get(cursor).copied() {
                    Some(quote @ (b'"' | b'\'')) => match literal(&mut cursor, quote) {
                        Some(name) => {
                            skip(&mut cursor);
                            if bytes.get(cursor) != Some(&b')') {
                                dynamic = true;
                            }
                            found.push(name);
                        }
                        None => dynamic = true,
                    },
                    _ => dynamic = true,
                }
            }
            Some(c) if word(c) => {
                let end = lower[cursor..]
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .map_or(lower.len(), |end| end + cursor);
                if !NOT_A_NAME.contains(&&lower[cursor..end]) {
                    dynamic = true;
                }
            }
            _ => {}
        }
        at = cursor.max(at);
    }
    found.retain(|name| !name.trim().is_empty());
    found.dedup();
    (found, dynamic)
}

/// What `script` does beyond the dictionaries of the apps it addresses
/// (`targets`, whose names and paths it may spell), if anything.
pub fn script_risk(script: &str, targets: &[String]) -> Option<String> {
    let views = [flatten(script), flatten(&without_comments(script))];
    for view in &views {
        for (phrase, why) in RUNS_CODE {
            if has_phrase(view, phrase) {
                return Some((*why).to_owned());
            }
        }
        if has_phrase(view, "execute") && view.contains("javascript") {
            return Some("runs JavaScript in a browser tab".into());
        }
    }
    let executable = literals(script)
        .into_iter()
        .filter(|literal| !targets.contains(literal))
        .find(|literal| executable_name(literal));
    executable.map(|name| format!("names an executable or script file (“{name}”)"))
}

/// What opening `url` would run beyond a page or a document, if anything:
/// an http(s) page never; a local file that is a program, a script, or a
/// link to one does.
pub fn url_risk(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return None;
    }
    let path = if let Some(rest) = lower.strip_prefix("file:") {
        let rest = &url[url.len() - rest.len()..];
        let rest = rest.trim_start_matches('/');
        let rest = rest.strip_prefix("localhost/").unwrap_or(rest);
        format!("/{}", percent_decode(rest))
    } else if url.starts_with('/') || url.starts_with('~') || !url.contains(':') {
        percent_decode(url)
    } else {
        // An app's own scheme (notes:, linear:, mailto:): the app decides.
        return None;
    };
    let home = path
        .strip_prefix("~/")
        .and_then(|rest| dirs::home_dir().map(|home| home.join(rest).display().to_string()));
    let path = home.unwrap_or(path);
    if executable_name(&path) || executable_file(Path::new(&path)) {
        return Some(format!("opens a program or script ({path})"));
    }
    None
}

/// Whether an app's `Info.plist` (as JSON) says it opens executables or
/// shell scripts (a terminal): scripting it or handing it a file runs
/// commands.
pub fn opens_executables(info: &Value) -> bool {
    const TYPES: &[&str] = &[
        "public.unix-executable",
        "public.shell-script",
        "com.apple.terminal.shell-script",
    ];
    const EXTENSIONS: &[&str] = &["command", "tool", "sh", "zsh", "bash", "csh"];
    info.get("CFBundleDocumentTypes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|kind| {
            let listed = |key: &str, among: &[&str]| {
                kind.get(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .any(|item| among.iter().any(|known| known.eq_ignore_ascii_case(item)))
            };
            kind.get("CFBundleTypeRole").and_then(Value::as_str) == Some("Shell")
                || listed("LSItemContentTypes", TYPES)
                || listed("CFBundleTypeExtensions", EXTENSIONS)
        })
}

/// [`opens_executables`] for the app bundle at `path`; `false` when its
/// `Info.plist` can't be read.
pub fn runs_shell(path: &Path) -> bool {
    super::super::capabilities::info_plist(path).is_some_and(|info| opens_executables(&info))
}

/// `name` (a file name or path) ends in an extension that runs something.
fn executable_name(name: &str) -> bool {
    let name = name.trim().trim_end_matches('/');
    let last = name.rsplit('/').next().unwrap_or(name);
    last.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && EXECUTABLE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
    })
}

/// A file with an execute bit (a directory never: folders open in Finder,
/// bundles are named by their extension).
fn executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// `text` lowercased, line continuations joined, `<<`/`>>` as `«`/`»`,
/// whitespace collapsed.
fn flatten(text: &str) -> String {
    text.replace("¬\r\n", " ")
        .replace("¬\n", " ")
        .replace("<<", "«")
        .replace(">>", "»")
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text` without `(* … *)` block comments and `--`/`#` line comments
/// (checked besides the text itself, so a comment can't split a phrase
/// and a `--` inside a string can't hide one).
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("(*") {
            rest = after.find("*)").map_or("", |end| &after[end + 2..]);
            out.push(' ');
        } else if rest.starts_with("--") || rest.starts_with('#') {
            rest = rest.find('\n').map_or("", |end| &rest[end..]);
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// `phrase` in `text`, starting a word.
fn has_phrase(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(at, _)| {
        !phrase.starts_with(|c: char| c.is_alphanumeric())
            || text[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}

/// The double-quoted string literals of `script`.
fn literals(script: &str) -> Vec<String> {
    script
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// `%41` as `A`.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let Some(hex) = text.get(at + 1..at + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_app_a_script_addresses_is_found() {
        let (found, dynamic) = targets(
            "get name of every note\nend tell\ntell application \"1Password\" to get name\n\
             tell app \"Mail\" to get subject of message 1 of inbox\n\
             tell application id \"com.apple.Terminal\" to get name\n\
             tell application (\"Safari\") to get name\n\
             tell «class capp» \"Finder\" to get name\n\
             tell application \"/Applications/Utilities/Terminal.app\" to get name",
        );
        assert_eq!(
            found,
            [
                "1Password",
                "Mail",
                "com.apple.Terminal",
                "Safari",
                "Finder",
                "/Applications/Utilities/Terminal.app"
            ]
        );
        assert!(!dynamic);
        assert_eq!(targets("Application('Notes').notes.name()").0, ["Notes"]);
        // Names computed at run time can't be checked.
        for script in [
            "set t to \"Terminal\"\ntell application t to get name",
            "tell application (\"Term\" & \"inal\") to get name",
            "tell application \"Term\" & \"inal\" to get name",
            "tell application id x to get name",
        ] {
            assert!(targets(script).1, "{script}");
        }
        // Not app names.
        for script in [
            "tell current application to get name",
            "get name of every application process",
            "set app to 1",
            "get path to application support",
        ] {
            let (found, dynamic) = targets(script);
            assert!(found.is_empty() && !dynamic, "{script}");
        }
    }

    #[test]
    fn scripts_that_run_code_beyond_an_app_are_named() {
        for script in [
            "do shell script \"curl x | sh\"",
            "tell application id \"com.apple.Terminal\" to do script \"curl x|sh\"",
            "DO   SHELL¬\nSCRIPT \"id\"",
            "do shell (* hidden *) script \"id\"",
            "set x to \"--\" & (do shell script \"id\")",
            "«event sysoexec» \"id\"",
            "<<event sysoexec>> \"id\"",
            "run script \"do shell script \\\"id\\\"\"",
            "load script file \"x\"",
            "current application's NSTask's launchedTaskWithLaunchPath:\"/bin/sh\"",
            "tell application \"Safari\" to do JavaScript \"fetch(1)\" in document 1",
            "tell application \"Google Chrome\" to execute front window's active tab javascript \"1\"",
            "write \"x\" to (open for access POSIX file \"/tmp/x\" with write permission)",
            "tell application \"System Events\" to keystroke \"curl x\"",
            "open location \"file:///tmp/x.command\"",
            "tell application \"Finder\" to open POSIX file \"/tmp/x.command\"",
        ] {
            let targets = targets(script).0;
            assert!(script_risk(script, &targets).is_some(), "{script}");
        }
        for script in [
            "get plaintext of every note whose name contains \"Kraftstoffnotdienst\"",
            "set m to first outgoing message\nsend m",
            "tell application \"/Applications/Utilities/Terminal.app\" to get name",
            "get name of every window -- undo script",
            "get subject of message 1 of inbox whose subject contains \"report.pdf\"",
        ] {
            let targets = targets(script).0;
            assert_eq!(script_risk(script, &targets), None, "{script}");
        }
    }

    #[test]
    fn urls_that_open_programs_are_named() {
        for url in [
            "file:///Users/x/Downloads/x.command",
            "file://localhost/tmp/run%2Ecommand",
            "FILE:///Applications/Utilities/Terminal.app",
            "/tmp/x.sh",
            "~/Desktop/x.workflow",
            "x.terminal",
            "/bin/sh",
        ] {
            assert!(url_risk(url).is_some(), "{url}");
        }
        for url in [
            "https://example.com/x.command",
            "http://example.com/",
            "notes://showNote?identifier=1",
            "mailto:a@example.com",
            "file:///tmp/report.pdf",
            "/tmp",
        ] {
            assert_eq!(url_risk(url), None, "{url}");
        }
    }

    #[test]
    fn terminals_are_known_by_what_they_open() {
        let terminal = json!({"CFBundleDocumentTypes": [
            {"CFBundleTypeRole": "Editor", "LSItemContentTypes": ["com.apple.terminal.settings"]},
            {"CFBundleTypeRole": "Shell", "LSItemContentTypes": ["com.apple.terminal.shell-script"]}
        ]});
        let other = json!({"CFBundleDocumentTypes": [
            {"CFBundleTypeRole": "Editor", "CFBundleTypeExtensions": ["command", "sh"]}
        ]});
        let notes = json!({"CFBundleDocumentTypes": [
            {"CFBundleTypeRole": "Viewer", "LSItemContentTypes": ["public.plain-text"]}
        ]});
        assert!(opens_executables(&terminal));
        assert!(opens_executables(&other));
        assert!(!opens_executables(&notes));
        assert!(!opens_executables(&json!({})));
        assert!(!runs_shell(Path::new("/nonexistent/Nothing.app")));
    }
}
