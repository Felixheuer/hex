//! What setup offers on a computer with no agent yet: the agents in the order a
//! new person should consider them, each with what it needs and its
//! one-click install.

use super::check::Fix;
use super::install::{Installers, Plan, Tool, plan};
use crate::app_settings::AssistantBackend;

/// One agent setup suggests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recommendation {
    pub backend: AssistantBackend,
    /// Its name, as the row's title.
    pub title: &'static str,
    /// Why to pick it, in a line.
    pub pitch: &'static str,
    /// What the person needs to use it.
    pub needs: &'static str,
    pub installed: bool,
    /// How it would be installed here (`Plan::Ready` when it is).
    pub install: Plan,
    /// The one-click fix that installs it; `None` once installed.
    pub fix: Option<Fix>,
}

/// The agents in the order setup recommends them for a new Mac. Asks what's
/// installed (seconds at most the first time); off the UI thread.
pub fn recommendations() -> Vec<Recommendation> {
    [
        AssistantBackend::ClaudeCode,
        AssistantBackend::OpenCode,
        AssistantBackend::Codex,
    ]
    .into_iter()
    .map(|backend| {
        let tool = Tool::Agent(backend);
        recommend(backend, plan(tool, &Installers::here(tool)))
    })
    .collect()
}

fn recommend(backend: AssistantBackend, install: Plan) -> Recommendation {
    let (title, pitch, needs) = match backend {
        AssistantBackend::ClaudeCode => (
            "Claude Code",
            "The best desktop agent. Uses your Claude subscription",
            "A Claude Pro, Max, Team, or Enterprise plan",
        ),
        AssistantBackend::OpenCode => (
            "OpenCode with your own key",
            "No subscription: bring a key from DeepSeek, OpenRouter, OpenAI, or another \
             provider and pay for what you use",
            "An API key from one of ten providers",
        ),
        AssistantBackend::Codex => (
            "Codex",
            "Uses your ChatGPT plan, or an OpenAI API key",
            "A ChatGPT plan or an OpenAI API key",
        ),
        AssistantBackend::ClaudeApi => (
            "Claude API",
            "Agent-Desk talks to Claude directly with your API key",
            "An Anthropic API key",
        ),
    };
    let installed = install == Plan::Ready;
    Recommendation {
        backend,
        title,
        pitch,
        needs,
        installed,
        fix: (!installed).then_some(Fix::Install(backend)),
        install,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn a_new_mac_gets_three_agents_each_with_its_install() {
        let bare = Installers {
            brew: Some(PathBuf::from("/opt/homebrew/bin/brew")),
            ..Installers::default()
        };
        let order = [
            AssistantBackend::ClaudeCode,
            AssistantBackend::OpenCode,
            AssistantBackend::Codex,
        ];
        let offered: Vec<_> = order
            .into_iter()
            .map(|backend| {
                let install = super::super::install::plan_for(
                    super::super::install::Os::Mac,
                    Tool::Agent(backend),
                    &bare,
                );
                recommend(backend, install)
            })
            .collect();
        assert_eq!(offered[0].title, "Claude Code");
        assert!(offered[1].pitch.contains("DeepSeek, OpenRouter"));
        for recommendation in &offered {
            assert!(!recommendation.installed);
            assert_eq!(
                recommendation.fix,
                Some(Fix::Install(recommendation.backend))
            );
            let Plan::Run(steps) = &recommendation.install else {
                panic!("no install plan for {}", recommendation.title);
            };
            assert!(
                steps[0]
                    .command_line_for(super::super::install::Os::Mac)
                    .starts_with("/opt/homebrew/bin/brew install")
            );
        }
        let installed = recommend(AssistantBackend::Codex, Plan::Ready);
        assert!(installed.installed);
        assert_eq!(installed.fix, None);
    }
}
