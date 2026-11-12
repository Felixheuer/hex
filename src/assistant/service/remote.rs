//! Requests from another Hex process (`hex assistant stop-all`, `take-over`,
//! `hand-back`) to whichever Hex runs the task: small files under
//! `<support>/assistant/requests/`, named `<target>.<verb>.<nonce>`, that
//! every runner with tasks looks at while it runs them. Each acts once per
//! file; the sender removes it after a moment.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use color_eyre::Result;

use crate::assistant::store;

/// How long a request counts; older files are ignored and cleared.
const FRESH: Duration = Duration::from_secs(10);

/// The target that means every task.
pub const ALL: &str = "all";

/// What another process asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verb {
    Stop,
    TakeOver,
    HandBack,
}

impl Verb {
    fn name(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::TakeOver => "take-over",
            Self::HandBack => "hand-back",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [Self::Stop, Self::TakeOver, Self::HandBack]
            .into_iter()
            .find(|verb| verb.name() == name)
    }
}

fn dir() -> Result<PathBuf> {
    let dir = store::assistant_dir()?.join("requests");
    crate::app_paths::create_private_dir(&dir)?;
    Ok(dir)
}

/// Asks every running Hex to do `verb` to `target`, then clears the request
/// after `linger`.
pub fn send(target: &str, verb: Verb, linger: Duration) -> Result<()> {
    if target != ALL
        && !target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        color_eyre::eyre::bail!("“{target}” is not a conversation id");
    }
    let path = dir()?.join(format!("{target}.{}.{}", verb.name(), store::new_id()));
    fs::write(&path, b"")?;
    std::thread::sleep(linger);
    let _ = fs::remove_file(&path);
    Ok(())
}

/// The requests not yet acted on (by `seen`, which keeps their names).
pub fn take_new(seen: &mut HashSet<String>) -> Vec<(String, Verb)> {
    let Ok(dir) = dir() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut present = HashSet::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        present.insert(name.clone());
        let fresh = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age < FRESH);
        if !fresh {
            let _ = fs::remove_file(entry.path());
            continue;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        let mut parts = name.splitn(3, '.');
        if let (Some(target), Some(verb)) = (parts.next(), parts.next().and_then(Verb::parse)) {
            found.push((target.to_owned(), verb));
        }
    }
    seen.retain(|name| present.contains(name));
    found
}
