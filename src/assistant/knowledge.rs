//! What Agent-Desk knows about the person's Mac, read locally and read-only:
//! `hex_lookup` finds their files (Spotlight), contacts, calendar,
//! reminders, mail (envelopes only), the tabs open in their browser and
//! their browsing history, apps, and the memory graph in one call, ranked,
//! in milliseconds; `hex_browser_tab` reads and acts in the tab the person
//! means in their own browser (whichever is their default); and the
//! memory graph ([`graph`]) links the people, companies, and projects the
//! person told Agent-Desk about, for every turn's context.
//!
//! Never message bodies, passwords, cookies, or form data; every answer is
//! bounded. Contacts and Reminders read only with the permission macOS
//! already gave (`knowledge.swift`); Mail, the browsers' history, and the
//! address book's fallback are read from copies with Full Disk Access.
//!
//! OWNER: knowledge.

mod agenda;
mod apple_events;
mod browser;
mod exec;
mod files;
pub mod graph;
mod history;
mod mail;
pub(crate) mod native;
mod people;
pub(crate) mod rank;
mod script;
mod tab;

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use rank::Hit;

pub use apple_events::person_fix;

pub const LOOKUP: &str = "hex_lookup";
pub const BROWSER_TAB: &str = tab::NAME;
/// The tools, in order: every role has both.
pub const NAMES: [&str; 2] = [LOOKUP, BROWSER_TAB];

/// How long an `all` lookup waits for its slowest source.
const ALL_DEADLINE: Duration = Duration::from_millis(3000);
const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 25;

/// A line for the system prompt: when to reach for these tools.
pub const RULES: &str = "\nThe person's own things: hex_lookup finds their files, contacts (numbers in international form), calendar, reminders, mail (subjects and senders), the tabs open in their browser and their browsing history, and what you know about people, companies, and projects (scope memory), in milliseconds. Use it first, before opening an app or searching the web for something of theirs. When the person means a tab they have open in their browser (\u{201c}my open tab\u{201d}, \u{201c}the page I'm on\u{201d}), use hex_browser_tab on that tab, never a fresh browser; to show a page in their browser (only when they ask for it there), navigate a tab with it, which stays in the background: never open their browser app or type into its address bar.\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Files,
    RecentFiles,
    Contacts,
    Calendar,
    Reminders,
    Mail,
    BrowserTabs,
    BrowserHistory,
    Memory,
    All,
}

impl Scope {
    pub const NAMES: [(&'static str, Scope); 10] = [
        ("files", Self::Files),
        ("recent_files", Self::RecentFiles),
        ("contacts", Self::Contacts),
        ("calendar", Self::Calendar),
        ("reminders", Self::Reminders),
        ("mail", Self::Mail),
        ("browser_tabs", Self::BrowserTabs),
        ("browser_history", Self::BrowserHistory),
        ("memory", Self::Memory),
        ("all", Self::All),
    ];

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_lowercase().replace([' ', '-'], "_");
        Self::NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, scope)| *scope)
    }

    pub fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, scope)| *scope == self)
            .map_or("all", |(name, _)| name)
    }

    /// What a result's time means.
    fn when_word(self) -> &'static str {
        match self {
            Self::Files | Self::RecentFiles => "used",
            Self::Mail => "received",
            Self::BrowserHistory => "visited",
            Self::Reminders => "due",
            _ => "",
        }
    }

    /// The scopes an `all` lookup asks, and how many results each gives.
    const ALL: [Scope; 8] = [
        Self::Memory,
        Self::Contacts,
        Self::Files,
        Self::Calendar,
        Self::Reminders,
        Self::Mail,
        Self::BrowserTabs,
        Self::BrowserHistory,
    ];
}

pub fn definitions() -> Vec<Value> {
    let scopes: Vec<&str> = Scope::NAMES.iter().map(|(name, _)| *name).collect();
    vec![
        json!({
            "name": LOOKUP,
            "description": "Find the person's own things on this Mac in milliseconds, locally and read-only: files (Spotlight: names, else contents), recent_files (used in the last two weeks), contacts (names, company, phone numbers in international form, emails), calendar (events 30 days back to 90 ahead), reminders (open ones), mail (subject, sender, date, mailbox; never bodies), browser_tabs (the tabs open in their default browser, with tab ids for hex_browser_tab), browser_history (titles, URLs, last visits, from every browser), memory (people, companies, projects and how they connect), or all of them at once. Results are ranked best first. Use it before opening an app or searching the web for something of theirs.",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string", "description": "Words to find (a name, a subject, part of a number or an address). May be empty for recent_files, calendar (what comes next), reminders, browser_tabs, or mail (the newest)."},
                "scope": {"type": "string", "enum": scopes, "description": "Where to look (default all)."},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT, "description": "Most results per source (default 8)."}
            }},
        }),
        tab::definition(),
    ]
}

pub fn handles(name: &str) -> bool {
    name == LOOKUP || name == BROWSER_TAB
}

/// Runs one of the knowledge tools for the task `task` (`--task`), for
/// `role` (a background job only reads the person's browser).
pub fn call(
    name: &str,
    arguments: &Value,
    task: Option<&str>,
    role: crate::assistant::tools::Role,
) -> Result<String, String> {
    match name {
        LOOKUP => {
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let scope = match arguments.get("scope").and_then(Value::as_str) {
                None | Some("") => Scope::All,
                Some(name) => Scope::parse(name).ok_or_else(|| {
                    format!("hex_lookup has no scope “{name}”; use one of files, recent_files, contacts, calendar, reminders, mail, browser_tabs, browser_history, memory, all")
                })?,
            };
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(DEFAULT_LIMIT, |limit| (limit as usize).clamp(1, MAX_LIMIT));
            if query.trim().is_empty()
                && matches!(
                    scope,
                    Scope::Files
                        | Scope::Contacts
                        | Scope::Memory
                        | Scope::BrowserHistory
                        | Scope::All
                )
            {
                return Err(format!(
                    "hex_lookup needs a “query” for scope {}",
                    scope.name()
                ));
            }
            // A source that failed is the call's error, with its real text
            // (the audit log keeps it, and the person sees their fix).
            if scope != Scope::All {
                return match search(scope, query, limit) {
                    Ok(hits) if hits.is_empty() => {
                        Ok(format!("Nothing found in {}.", scope.name()))
                    }
                    Ok(hits) => Ok(format_hits(scope, &hits)),
                    Err(error) => Err(format!("{} couldn't be searched: {error}", scope.name())),
                };
            }
            Ok(lookup(query, scope, limit))
        }
        BROWSER_TAB => tab::call(arguments, task, role),
        other => Err(format!("There is no tool “{other}”.")),
    }
}

/// One source's results, or why it gave none.
fn search(scope: Scope, query: &str, limit: usize) -> Result<Vec<Hit>, String> {
    let tokens = rank::tokens(query);
    let now = crate::assistant::store::now_ms() as i64;
    match scope {
        Scope::Files => files::files(&tokens, limit, now),
        Scope::RecentFiles => files::recent(&tokens, limit),
        Scope::Contacts => people::contacts(query, &tokens, limit),
        Scope::Calendar => agenda::calendar(&tokens, limit, now),
        Scope::Reminders => agenda::reminders(query, &tokens, limit),
        Scope::Mail => mail::mail(query, limit, now),
        Scope::BrowserTabs => browser::tabs(&tokens, limit),
        Scope::BrowserHistory => history::history(query, limit, now),
        Scope::Memory => memory(query, limit),
        Scope::All => Err("not a single source".into()),
    }
}

fn memory(query: &str, limit: usize) -> Result<Vec<Hit>, String> {
    if !memory_on() {
        return Err("the person turned memory off".into());
    }
    let (_, graph) = graph::load().ok_or("Agent-Desk has no memory folder")?;
    Ok(graph
        .search(query, limit)
        .into_iter()
        .map(|(score, line)| Hit {
            title: line,
            detail: String::new(),
            when_ms: None,
            score,
        })
        .collect())
}

pub fn memory_on() -> bool {
    crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.memory)
        .or_else(|| {
            crate::app_settings::AppSettings::read()
                .ok()
                .map(|settings| settings.assistant.memory)
        })
        .unwrap_or(true)
}

/// A source's results as lines.
pub fn format_hits(scope: Scope, hits: &[Hit]) -> String {
    hits.iter()
        .map(|hit| {
            let mut line = format!("- {}", hit.title);
            if !hit.detail.is_empty() {
                line.push_str(" — ");
                line.push_str(&hit.detail);
            }
            if let Some(when) = hit.when_ms {
                let word = scope.when_word();
                line.push_str(" — ");
                if !word.is_empty() {
                    line.push_str(word);
                    line.push(' ');
                }
                line.push_str(&rank::local_time(when));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every source's section, in `Scope::ALL` order; sources that found
/// nothing are left out, sources that failed are named at the end.
pub fn merge(results: Vec<(Scope, Result<Vec<Hit>, String>)>) -> String {
    let mut sections = Vec::new();
    let mut failed = Vec::new();
    for (scope, result) in results {
        match result {
            Ok(hits) if hits.is_empty() => {}
            Ok(hits) => sections.push(format!("{}:\n{}", scope.name(), format_hits(scope, &hits))),
            Err(error) => failed.push(format!("{}: {error}", scope.name())),
        }
    }
    let mut text = if sections.is_empty() {
        "Nothing found.".to_owned()
    } else {
        sections.join("\n\n")
    };
    if !failed.is_empty() {
        text.push_str("\n\nNot searched: ");
        text.push_str(&failed.join("; "));
    }
    text
}

/// The lookup's answer for the agent.
pub fn lookup(query: &str, scope: Scope, limit: usize) -> String {
    if scope != Scope::All {
        return match search(scope, query, limit) {
            Ok(hits) if hits.is_empty() => format!("Nothing found in {}.", scope.name()),
            Ok(hits) => format_hits(scope, &hits),
            Err(error) => format!("{} couldn't be searched: {error}", scope.name()),
        };
    }
    merge(search_all(query, limit.min(5)))
}

/// Every source at once, each in its own thread; one still running at the
/// deadline is reported as too slow.
fn search_all(query: &str, limit: usize) -> Vec<(Scope, Result<Vec<Hit>, String>)> {
    let (sender, receiver) = std::sync::mpsc::channel();
    for scope in Scope::ALL {
        let sender = sender.clone();
        let query = query.to_owned();
        std::thread::spawn(move || {
            let _ = sender.send((scope, search(scope, &query, limit)));
        });
    }
    drop(sender);
    let deadline = Instant::now() + ALL_DEADLINE;
    let mut results: Vec<(Scope, Result<Vec<Hit>, String>)> = Vec::new();
    while results.len() < Scope::ALL.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(result) => results.push(result),
            Err(_) => break,
        }
    }
    for scope in Scope::ALL {
        if !results.iter().any(|(done, _)| *done == scope) {
            results.push((scope, Err("too slow this time".into())));
        }
    }
    results.sort_by_key(|(scope, _)| Scope::ALL.iter().position(|known| known == scope));
    results
}

/// What the person's memory graph knows about the people, companies, and
/// projects `request` names, for the request's message
/// (`agent::request_rules`), never the system prompt: that stays the same
/// whatever the words, so a backend started ahead and the prompt cache keep
/// it. `None` when memory is off or the words name nothing it knows (one
/// hook in `service::run`).
pub fn recall(request: &str, memory: bool) -> Option<String> {
    if !memory {
        return None;
    }
    recall_in(&graph::load()?.1, request)
}

/// [`recall`] from `graph`.
pub fn recall_in(graph: &graph::Graph, request: &str) -> Option<String> {
    graph.context(request)
}

/// Adds a remembered fact to the graph (`hex_remember`), `about` naming
/// whom or what it is about when the agent said so.
pub fn learn_fact(fact: &str, about: Option<&str>) {
    let Some((path, graph)) = graph::load() else {
        return;
    };
    let region = crate::assistant::phone::incoming::region();
    learn_fact_into(&path, graph, fact, about, &region);
}

/// [`learn_fact`] into `graph`, kept at `path`, for a person in `region`.
pub fn learn_fact_into(
    path: &std::path::Path,
    mut graph: graph::Graph,
    fact: &str,
    about: Option<&str>,
    region: &str,
) {
    let fact = graph::extract(fact, about, region);
    if graph.learn(&fact, "remember", crate::assistant::store::now_ms())
        && let Err(error) = graph::save_to(path, &graph)
    {
        tracing::warn!(%error, "couldn't save the memory graph");
    }
}

/// Learns from a finished task's answer, off the caller's thread, while
/// memory is on (the `learn` hook in `service::run` asks only then).
pub fn learn_from_task(conversation: &str, request: &str, answer: &str) {
    let (conversation, request, answer) = (
        conversation.to_owned(),
        request.to_owned(),
        answer.to_owned(),
    );
    std::thread::spawn(move || {
        let Some((path, mut graph)) = graph::load() else {
            return;
        };
        let now = crate::assistant::store::now_ms();
        if graph.learn_from_task(&conversation, &request, &answer, now)
            && let Err(error) = graph::save_to(&path, &graph)
        {
            tracing::warn!(%error, "couldn't save the memory graph");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::tools::Role;

    #[test]
    fn scopes_parse_and_bad_calls_say_why() {
        assert_eq!(Scope::parse("Browser tabs"), Some(Scope::BrowserTabs));
        assert_eq!(Scope::parse("browser-history"), Some(Scope::BrowserHistory));
        assert_eq!(Scope::parse("chrome_tabs"), None);
        assert_eq!(Scope::parse("recent-files"), Some(Scope::RecentFiles));
        assert_eq!(Scope::parse("x"), None);
        for (name, scope) in Scope::NAMES {
            assert_eq!(scope.name(), name);
        }
        assert!(
            call(
                LOOKUP,
                &json!({"query": "x", "scope": "web"}),
                None,
                Role::Turn
            )
            .unwrap_err()
            .contains("no scope")
        );
        assert!(
            call(LOOKUP, &json!({"scope": "files"}), None, Role::Turn)
                .unwrap_err()
                .contains("needs a “query”")
        );
        assert!(handles("hex_lookup") && handles("hex_browser_tab") && !handles("hex_remember"));
        assert!(!handles("hex_chrome_tab"));
        let tools = definitions();
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(names, NAMES);
        // Installed apps are the guard's hex_installed_apps: one tool per job.
        assert_eq!(Scope::parse("apps"), None);
    }

    #[test]
    fn a_remembered_fact_is_saved_into_the_graph_and_recalled() {
        let dir = std::env::temp_dir().join(format!("hex-knowledge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory-graph.json");
        learn_fact_into(
            &path,
            graph::Graph::default(),
            "Company: Kalibermarkt",
            None,
            "DE",
        );
        let saved = graph::load_from(&path, String::new, "DE");
        let _ = std::fs::remove_dir_all(&dir);
        let recalled = recall_in(&saved, "Wie läuft Kalibermarkt?").unwrap_or_default();
        assert!(recalled.contains("Kalibermarkt"), "{recalled}");
        assert_eq!(recall_in(&saved, "Wie wird das Wetter?"), None);
    }

    #[test]
    fn results_read_as_compact_sections() {
        let hit = |title: &str, detail: &str, when| Hit {
            title: title.into(),
            detail: detail.into(),
            when_ms: when,
            score: 1.0,
        };
        let text = merge(vec![
            (
                Scope::Contacts,
                Ok(vec![hit("Michael Schwarz", "mobile +491621234567", None)]),
            ),
            (Scope::Files, Ok(vec![])),
            (Scope::Mail, Err("needs Full Disk Access".into())),
            (Scope::Calendar, Ok(vec![hit("Standup", "Team", None)])),
        ]);
        assert_eq!(
            text,
            "contacts:\n- Michael Schwarz — mobile +491621234567\n\ncalendar:\n- Standup — Team\n\nNot searched: mail: needs Full Disk Access"
        );
        assert_eq!(merge(vec![(Scope::Files, Ok(vec![]))]), "Nothing found.");
        let dated = format_hits(Scope::Mail, &[hit("Rechnung", "from x", Some(0))]);
        assert!(
            dated.starts_with("- Rechnung — from x — received 19"),
            "{dated}"
        );
    }

    #[test]
    fn the_graph_recalls_what_the_words_name_and_nothing_when_memory_is_off() {
        assert_eq!(recall("Michael", false), None);
        let mut graph = graph::Graph::default();
        graph.learn(
            &graph::extract("Michael Schwarz works at Kalibermarkt", None, "DE"),
            "remember",
            1,
        );
        let recalled = recall_in(&graph, "Schreib Michael Schwarz").unwrap();
        assert!(recalled.contains("- Michael Schwarz ("), "{recalled}");
        assert_eq!(recall_in(&graph, "Wie wird das Wetter?"), None);
    }

    /// Measures each source on this Mac (read-only):
    /// `cargo test --bin agent-desk knowledge::tests::measure -- --ignored --nocapture`.
    /// The browser's tabs only with AGENT_DESK_MEASURE_BROWSER_TABS=1 (it may ask
    /// for Automation permission).
    #[test]
    #[ignore = "reads this Mac's own data"]
    fn measure() {
        let query = std::env::var("AGENT_DESK_MEASURE_QUERY").unwrap_or_else(|_| "rechnung".into());
        for scope in Scope::ALL.into_iter().chain([Scope::RecentFiles]) {
            if scope == Scope::BrowserTabs
                && std::env::var("AGENT_DESK_MEASURE_BROWSER_TABS").is_err()
            {
                continue;
            }
            let query = if scope == Scope::RecentFiles {
                ""
            } else {
                query.as_str()
            };
            let start = Instant::now();
            let result = search(scope, query, 8);
            let took = start.elapsed();
            match result {
                Ok(hits) => println!(
                    "{:>15}: {:>5} ms, {} results",
                    scope.name(),
                    took.as_millis(),
                    hits.len()
                ),
                Err(error) => println!(
                    "{:>15}: {:>5} ms, error: {error}",
                    scope.name(),
                    took.as_millis()
                ),
            }
        }
        let start = Instant::now();
        let all = search_all(&query, 5);
        println!(
            "{:>15}: {:>5} ms, {} sources",
            "all",
            start.elapsed().as_millis(),
            all.len()
        );
    }
}
