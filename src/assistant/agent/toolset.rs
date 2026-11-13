//! The tools a run has, per lane and backend, in one place: the guard's
//! desktop tools (`guard::proxy::tools`), Agent-Desk's own (`tools::Role`), the
//! hidden browser's, and the backend's built-ins. The rules name only
//! these, by the name each backend gives them, and the tests check that
//! every lane's rules and toolset agree for every backend.

use crate::app_settings::AssistantBackend;

/// A backend's built-in tools that the rules name.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Builtins {
    /// Fetches a known URL as text.
    pub fetch: Option<&'static str>,
    /// Searches the web.
    pub search: Option<&'static str>,
    /// Runs subagents for independent batches (a job's).
    pub subagents: Option<&'static str>,
}

impl Builtins {
    /// What `backend` has built in, for a job (`job`) or a quick turn.
    /// Codex runs only its shell; the Claude API only the servers' tools.
    pub fn of(backend: AssistantBackend, job: bool) -> Self {
        match backend {
            // `provider::claude_code`: web search and fetch always, files,
            // shell, and Task in a job's workspace.
            AssistantBackend::ClaudeCode => Self {
                fetch: Some("WebFetch"),
                search: Some("WebSearch"),
                subagents: job.then_some("Task"),
            },
            // `provider::opencode`: webfetch is allowed in every run, its
            // task tool runs subagents.
            AssistantBackend::OpenCode => Self {
                fetch: Some("webfetch"),
                search: None,
                subagents: job.then_some("task"),
            },
            AssistantBackend::Codex | AssistantBackend::ClaudeApi => Self::default(),
        }
    }

    /// Their names.
    pub fn names(self) -> Vec<&'static str> {
        [self.fetch, self.search, self.subagents]
            .into_iter()
            .flatten()
            .collect()
    }
}

/// Every built-in name any backend has: the rules may name one only where
/// that backend has it.
pub const ALL_BUILTINS: [&str; 5] = ["WebFetch", "WebSearch", "Task", "webfetch", "task"];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::app_settings::AssistantSettings;
    use crate::assistant::agent::{Job, JobKind, system_for, task_spec, tool_role};
    use crate::assistant::provider::Desktop;
    use crate::assistant::types::{Context, Lane, Origin, Request};

    /// One kind of run: who asks, what the settings allow, and what Agent-Desk
    /// gives it.
    #[derive(Clone, Copy)]
    struct Variant {
        name: &'static str,
        origin: Origin,
        text: &'static str,
        desktop: bool,
        full_control: bool,
        browser: bool,
        workspace: bool,
        helps: bool,
        memory: bool,
    }

    const BACKENDS: [AssistantBackend; 4] = [
        AssistantBackend::ClaudeCode,
        AssistantBackend::Codex,
        AssistantBackend::OpenCode,
        AssistantBackend::ClaudeApi,
    ];

    const HERE: Variant = Variant {
        name: "here, full control",
        origin: Origin::ActionShortcut,
        text: "Was steht in dieser Mail?",
        desktop: true,
        full_control: true,
        browser: true,
        workspace: false,
        helps: false,
        memory: true,
    };

    const JOB: Variant = Variant {
        name: "job",
        origin: Origin::Background,
        workspace: true,
        full_control: false,
        ..HERE
    };

    const HELPER: Variant = Variant {
        name: "helper",
        helps: true,
        ..JOB
    };

    fn variants() -> Vec<Variant> {
        vec![
            Variant {
                name: "here, no desktop, memory off",
                desktop: false,
                memory: false,
                ..HERE
            },
            Variant {
                name: "here, the window in front only",
                full_control: false,
                ..HERE
            },
            HERE,
            Variant {
                name: "here, only in the background",
                text: "Kannst du jede beliebige App nur im Hintergrund bedienen?",
                ..HERE
            },
            Variant {
                name: "chat",
                origin: Origin::Chat,
                full_control: false,
                ..HERE
            },
            JOB,
            Variant {
                name: "job, no desktop or browser",
                desktop: false,
                browser: false,
                ..JOB
            },
            HELPER,
        ]
    }

    /// The run `variant` is on `backend`: its system prompt, every tool it
    /// is offered, and Agent-Desk's own among them (the ones its rules must name).
    fn run(
        variant: &Variant,
        backend: AssistantBackend,
    ) -> (String, BTreeSet<String>, BTreeSet<String>) {
        let settings = AssistantSettings {
            backend,
            control_desktop: variant.desktop,
            full_control: variant.full_control,
            memory: variant.memory,
            ..Default::default()
        };
        let job = Job {
            conversation: "c1".into(),
            request: Request {
                conversation: None,
                text: variant.text.into(),
                origin: variant.origin,
                context: Context::default(),
            },
            settings,
            kind: JobKind::Ask,
            digest: None,
            workspace: variant.workspace.then(|| "/tmp/job".into()),
            authorized: Default::default(),
            memory: Some("- Company: blabla.com".into()),
            recall: None,
            project: None,
            helps: variant.helps.then(|| "Lead".into()),
        };
        let spec = task_spec(&job);
        // As `agent::prepare` gives them.
        let desktop = variant.desktop.then(|| {
            let mut desktop = Desktop::fixture();
            desktop.front_window = spec.full_control;
            desktop.open_apps = spec.open_apps;
            desktop
        });
        let browser = variant.browser && (spec.lane == Lane::Background || spec.full_control);
        let system = system_for(&job, desktop.as_ref(), browser);
        let mut hex = BTreeSet::new();
        let mut offered = BTreeSet::new();
        if desktop.is_some() {
            for tool in crate::assistant::guard::offered_tools(&spec) {
                if tool.starts_with("hex_") || tool == crate::assistant::guard::CLAIM_TOOL {
                    hex.insert(tool.clone());
                }
                offered.insert(tool);
            }
        }
        for tool in tool_role(&job).tools(variant.memory) {
            hex.insert(tool.to_owned());
            offered.insert(tool.to_owned());
        }
        if browser {
            offered.insert(crate::assistant::browser::sessions::LIST_TOOL.to_owned());
        }
        for tool in Builtins::of(backend, variant.workspace).names() {
            offered.insert(tool.to_owned());
        }
        (system, offered, hex)
    }

    /// Every `hex_*` and `desktop_*` name in `text`.
    fn named(text: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for prefix in ["hex_", "desktop_"] {
            for (at, _) in text.match_indices(prefix) {
                let before = text[..at].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                    continue;
                }
                let name: String = text[at..]
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                    .collect();
                names.insert(name.trim_end_matches('_').to_owned());
            }
        }
        names
    }

    /// The built-ins `text` names, as the rules write them.
    fn named_builtins(text: &str) -> BTreeSet<&'static str> {
        ALL_BUILTINS
            .into_iter()
            .filter(|tool| match *tool {
                "Task" | "task" => text.contains(&format!("the {tool} tool")),
                tool => text
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| word == tool),
            })
            .collect()
    }

    /// Every tool any lane's rules name is one that lane has, on every
    /// backend; and every tool of Agent-Desk's that a lane has, its rules name.
    #[test]
    fn every_lane_s_rules_and_tools_agree_on_every_backend() {
        for variant in variants() {
            for backend in BACKENDS {
                let (system, offered, hex) = run(&variant, backend);
                let at = format!("{} on {backend:?}", variant.name);
                let named = named(&system);
                for name in &named {
                    assert!(
                        offered.contains(name),
                        "{at}: the rules name {name}, which it doesn't have"
                    );
                }
                for tool in named_builtins(&system) {
                    assert!(
                        offered.contains(tool),
                        "{at}: the rules name {tool}, which {backend:?} doesn't have"
                    );
                }
                for tool in &hex {
                    assert!(
                        named.contains(tool),
                        "{at}: it has {tool}, but its rules never name it"
                    );
                }
                // One tool per job.
                assert!(!offered.contains("desktop_launch"), "{at}");
                assert!(!offered.contains("hex_displays"), "{at}");
            }
        }
    }

    /// The work-fast rules reach every role; schedules are the turn's.
    #[test]
    fn jobs_work_fast_too_and_only_turns_schedule() {
        for variant in variants() {
            let (system, offered, _) = run(&variant, AssistantBackend::ClaudeCode);
            assert!(system.contains("Work fast"), "{}", variant.name);
            assert_eq!(
                system.contains("hex_schedule"),
                offered.contains("hex_schedule"),
                "{}",
                variant.name
            );
        }
        let (job, ..) = run(&JOB, AssistantBackend::ClaudeCode);
        assert!(job.contains("WebFetch for a known URL") && job.contains("the Task tool"));
        let (codex, ..) = run(&JOB, AssistantBackend::Codex);
        assert!(!codex.contains("WebFetch") && !codex.contains("the Task tool"));
        let (opencode, ..) = run(&JOB, AssistantBackend::OpenCode);
        assert!(
            opencode.contains("webfetch for a known URL") && opencode.contains("the task tool")
        );
    }

    /// A helper does its part: no team rules, no team tools, and no rule
    /// that tells it both to start helpers and not to.
    #[test]
    fn a_helper_gets_no_team() {
        let (system, offered, _) = run(&HELPER, AssistantBackend::ClaudeCode);
        assert!(!system.contains("hex_team_start"), "{system}");
        assert!(!offered.contains("hex_team_start"));
        assert!(system.contains("You are a helper"));
        let (lead, offered, _) = run(&JOB, AssistantBackend::ClaudeCode);
        assert!(lead.contains("hex_team_start") && offered.contains("hex_team_start"));
    }

    /// The web has one routing rule: a quick look is the turn's, research
    /// across sources a job's.
    #[test]
    fn the_browser_is_routed_in_one_place() {
        let (system, ..) = run(&HERE, AssistantBackend::ClaudeCode);
        assert_eq!(
            system
                .matches("research across several sources is a job")
                .count(),
            1,
            "{system}"
        );
        assert!(!system.contains("for a quick look on the web"), "{system}");
    }
}
