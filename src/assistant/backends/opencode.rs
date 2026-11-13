//! Which OpenCode is installed, and what each one takes.
//!
//! Two programs answer to `opencode`:
//! - **OpenCode 2** (`@opencode/cli`, `anomalyco/tap/opencode-v2`, the
//!   `opencode.ai/v2` installer; dev and beta builds are `0.0.0-…`), which
//!   Agent-Desk installs. Its `run` takes `--standalone` (a private server
//!   for this one task) and `--archived` (the session stays out of the
//!   person's list), and its config names providers `providers.<id>.settings`.
//! - **Classic OpenCode** (1.x and the 0.x before it: `opencode-ai` on npm,
//!   `~/.opencode/bin`, Homebrew's `opencode`), which many people already
//!   have. Its `run` already runs in its own process and knows neither
//!   flag (it prints its help and fails), and its config names providers
//!   `provider.<id>.options` and rejects `providers`.
//!
//! Which one a path is, is read from what it offers: `opencode run --help`
//! lists `--standalone` only on OpenCode 2 ([`generation_of_help`]). The
//! version alone can't tell: both publish `0.0.0-dev-…` and `0.0.0-beta-…`
//! builds (classic numbers them by date, `0.0.0-dev-202611092046`; OpenCode 2
//! by run, `0.0.0-dev-1289`), so it is only the fallback when the help
//! doesn't answer ([`generation_of`]).
//!
//! Tasks, self-tests and the model list ask [`generation`] and build their
//! arguments and config here, so both work.
//!
//! OWNER: brain.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::{Map, Value};

use crate::app_settings::AssistantBackend;

/// Which OpenCode a path is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Generation {
    /// 1.x and older: no private-server or archive flags.
    Classic,
    /// OpenCode 2, the one Agent-Desk installs.
    Two,
}

/// The generation `opencode run --help`'s words (both streams) say.
pub fn generation_of_help(help: &str) -> Option<Generation> {
    if !help.contains("--format") {
        // Not `run`'s help at all.
        return None;
    }
    Some(if help.contains("--standalone") {
        Generation::Two
    } else {
        Generation::Classic
    })
}

/// The generation a `--version` number belongs to; an unknown version is
/// taken for OpenCode 2, which Agent-Desk installs.
pub fn generation_of(version: Option<&str>) -> Generation {
    let Some(version) = version.map(str::trim).filter(|version| !version.is_empty()) else {
        return Generation::Two;
    };
    // Snapshot builds: classic's carry a date (`0.0.0-202506070331`,
    // `0.0.0-dev-202611092046`), OpenCode 2's a run number
    // (`0.0.0-dev-1289`, `0.0.0-beta-19507`).
    if let Some(snapshot) = version.strip_prefix("0.0.0-") {
        let number = snapshot.rsplit('-').next().unwrap_or_default();
        let dated = number.len() >= 12 && number.chars().all(|c| c.is_ascii_digit());
        return if dated {
            Generation::Classic
        } else {
            Generation::Two
        };
    }
    match version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
    {
        Some(major) if major >= 2 => Generation::Two,
        Some(_) => Generation::Classic,
        None => Generation::Two,
    }
}

/// The generation of the OpenCode tasks run, remembered per executable
/// until it changes on disk (an update), so a task doesn't wait on its
/// help each time.
pub fn generation() -> Generation {
    match super::executable(AssistantBackend::OpenCode) {
        Some(path) => generation_for(&path),
        None => Generation::Two,
    }
}

/// [`generation`] for `path`.
pub fn generation_for(path: &Path) -> Generation {
    static SEEN: Mutex<Option<(PathBuf, Option<SystemTime>, Generation)>> = Mutex::new(None);
    let modified = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok();
    if let Ok(seen) = SEEN.lock()
        && let Some((seen_path, seen_modified, generation)) = seen.as_ref()
        && seen_path == path
        && *seen_modified == modified
    {
        return *generation;
    }
    let mut command = super::agent_command(path);
    command.args(["run", "--help"]);
    let help = super::run::bounded(command, std::time::Duration::from_secs(10))
        .ok()
        .filter(|ran| !ran.timed_out())
        .and_then(|ran| generation_of_help(&format!("{}\n{}", ran.stdout, ran.stderr)));
    let Some(generation) = help else {
        // A slow or failing start isn't remembered: the next task asks
        // again. Meanwhile the version guesses.
        let version = super::version_of(path).ok().flatten();
        return generation_of(version.as_deref());
    };
    if let Ok(mut seen) = SEEN.lock() {
        *seen = Some((path.to_owned(), modified, generation));
    }
    generation
}

/// `opencode run`'s arguments before the agent, model and message: JSON
/// events, nothing waiting on a permission prompt, and (OpenCode 2) a
/// private server and an archived session.
pub fn run_args(generation: Generation) -> Vec<&'static str> {
    match generation {
        Generation::Two => vec![
            "run",
            "--standalone",
            "--format",
            "json",
            "--auto",
            "--archived",
        ],
        // Its sessions show in the person's list; the title says whose.
        Generation::Classic => vec!["run", "--format", "json", "--auto", "--title", "Agent-Desk"],
    }
}

/// `opencode models` with the person's own sign-ins, without waiting on a
/// background service.
pub fn models_args(generation: Generation) -> Vec<&'static str> {
    match generation {
        Generation::Two => vec!["models", "--standalone"],
        Generation::Classic => vec!["models"],
    }
}

/// `config` (an `OPENCODE_CONFIG_CONTENT` object, written in OpenCode 2's
/// words) in `generation`'s words: Classic OpenCode's providers are
/// `provider.<id>.options`, and it rejects the whole config for an
/// unknown `providers`.
pub fn adapt_config(config: &mut Value, generation: Generation) {
    if generation == Generation::Two {
        return;
    }
    let Some(object) = config.as_object_mut() else {
        return;
    };
    let Some(Value::Object(providers)) = object.remove("providers") else {
        return;
    };
    let mut classic = match object.remove("provider") {
        Some(Value::Object(existing)) => existing,
        _ => Map::new(),
    };
    for (id, mut entry) in providers {
        if let Some(entry) = entry.as_object_mut()
            && let Some(settings) = entry.remove("settings")
        {
            entry.insert("options".into(), settings);
        }
        match classic.get_mut(&id) {
            Some(existing) => super::merge_config(existing, &entry),
            None => {
                classic.insert(id, entry);
            }
        }
    }
    object.insert("provider".into(), Value::Object(classic));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_version_tells_the_two_opencodes_apart() {
        for (version, generation) in [
            (Some("1.19.5"), Generation::Classic),
            (Some("0.15.8"), Generation::Classic),
            (Some("2.3.0"), Generation::Two),
            (Some("2.2.6"), Generation::Two),
            (Some("0.0.0-dev-1289"), Generation::Two),
            (Some("0.0.0-beta-19507"), Generation::Two),
            (Some("0.0.0-dev-202611092046"), Generation::Classic),
            (Some("0.0.0-beta-202601310003"), Generation::Classic),
            (Some("0.0.0-202506070331"), Generation::Classic),
            (None, Generation::Two),
            (Some(""), Generation::Two),
        ] {
            assert_eq!(generation_of(version), generation, "{version:?}");
        }
        assert_eq!(
            generation_of(crate::assistant::backends::parse_version("opencode v2.3.0").as_deref()),
            Generation::Two
        );
        assert_eq!(
            generation_of(crate::assistant::backends::parse_version("1.19.5\n").as_deref()),
            Generation::Classic
        );
    }

    #[test]
    fn the_help_says_which_opencode_it_is() {
        let two = "USAGE\n  opencode run [flags] [<message...>]\nFLAGS\n  --format choice  Output format\n  --archived\nGLOBAL FLAGS\n  --standalone  Run with a private server";
        let classic = "opencode run [message..]\n\nOptions:\n  --format  format: default (formatted) or json\n  --auto  auto-approve permissions";
        assert_eq!(generation_of_help(two), Some(Generation::Two));
        assert_eq!(generation_of_help(classic), Some(Generation::Classic));
        assert_eq!(generation_of_help("error: unknown command"), None);
        assert_eq!(generation_of_help(""), None);
    }

    #[test]
    fn classic_opencode_gets_no_flags_it_does_not_know() {
        let classic = run_args(Generation::Classic);
        assert!(!classic.contains(&"--standalone"));
        assert!(!classic.contains(&"--archived"));
        assert!(classic.contains(&"--auto"));
        let two = run_args(Generation::Two);
        assert!(two.contains(&"--standalone") && two.contains(&"--archived"));
        assert_eq!(models_args(Generation::Classic), ["models"]);
        assert_eq!(models_args(Generation::Two), ["models", "--standalone"]);
    }

    #[test]
    fn classic_opencode_gets_its_providers_in_its_own_words() {
        let two = json!({
            "model": "deepseek/deepseek-v4-pro",
            "providers": {"deepseek": {"settings": {"apiKey": "{env:DEEPSEEK_API_KEY}"}}},
            "agent": {"hex": {"mode": "primary"}},
        });
        let mut same = two.clone();
        adapt_config(&mut same, Generation::Two);
        assert_eq!(same, two);

        let mut classic = two.clone();
        adapt_config(&mut classic, Generation::Classic);
        assert_eq!(
            classic,
            json!({
                "model": "deepseek/deepseek-v4-pro",
                "provider": {"deepseek": {"options": {"apiKey": "{env:DEEPSEEK_API_KEY}"}}},
                "agent": {"hex": {"mode": "primary"}},
            })
        );
        // Nothing to change: unchanged.
        let mut plain = json!({"mcp": {}});
        adapt_config(&mut plain, Generation::Classic);
        assert_eq!(plain, json!({"mcp": {}}));
    }
}
