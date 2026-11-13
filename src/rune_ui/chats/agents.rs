//! Choosing and setting up the agent, for the onboarding's agent page and
//! Settings alike: each agent's one action does everything its state needs
//! (install, then sign in or start, then check it answers), with its
//! installer's lines as it goes; OpenCode runs on the person's own key with
//! any provider (installed first when it isn't); Codex can take an OpenAI
//! key instead of a sign-in.

use rune::TextEdit;

use super::{BACKENDS, Done, Job, Ui};
use crate::app_settings::{AssistantBackend, AssistantSettings};
use crate::assistant::backends::{self, Check, Fix, Found, Provider};

/// How many of a setup's latest lines it keeps.
const LOG: usize = 6;

/// The own-key panel for OpenCode.
#[derive(Default)]
pub(in crate::rune_ui) struct Byok {
    /// Into [`backends::providers`].
    pub(in crate::rune_ui) provider: usize,
    /// Into that provider's models.
    pub(in crate::rune_ui) model: usize,
    pub(in crate::rune_ui) key: TextEdit,
    pub(in crate::rune_ui) verifying: bool,
    /// The last check of the key: the model it saved, or why not.
    pub(in crate::rune_ui) result: Option<Result<String, String>>,
}

/// The agent picker's state.
#[derive(Default)]
pub(in crate::rune_ui) struct Agents {
    /// The agent whose setup runs now.
    pub(in crate::rune_ui) working: Option<AssistantBackend>,
    /// Its latest lines.
    pub(in crate::rune_ui) log: Vec<String>,
    /// The last setup that failed, why, and its Terminal command.
    pub(in crate::rune_ui) failed: Option<(AssistantBackend, String)>,
    pub(in crate::rune_ui) command: Option<String>,
    /// OpenCode's own-key panel, while open.
    pub(in crate::rune_ui) byok: Option<Byok>,
    /// The Claude API key field, while open.
    pub(in crate::rune_ui) api_key_open: bool,
    /// Codex's OpenAI key field, while open, and its last result.
    pub(in crate::rune_ui) codex_key: Option<TextEdit>,
    pub(in crate::rune_ui) codex_error: Option<String>,
    /// With nothing installed: the whole list instead of the suggestion.
    pub(in crate::rune_ui) show_all: bool,
}

/// Where an agent stands, as its chip says.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Status {
    Ready,
    SignIn,
    Install,
    AddKey,
    /// Not looked for yet.
    Unknown,
}

pub(in crate::rune_ui) fn status(found: Option<&Found>) -> Status {
    let Some(found) = found else {
        return Status::Unknown;
    };
    if found.ready {
        return Status::Ready;
    }
    match super::setup::fix_for(found) {
        Some(Fix::AddApiKey | Fix::UseApiKey(_)) => Status::AddKey,
        Some(Fix::Install(_)) => Status::Install,
        // OpenCode installed but not running answers on its own key too.
        _ if found.backend == AssistantBackend::OpenCode => Status::AddKey,
        _ => Status::SignIn,
    }
}

/// `backend` as found, once looked for.
pub(in crate::rune_ui) fn found(ui: &Ui, backend: AssistantBackend) -> Option<&Found> {
    ui.chats
        .settings
        .found
        .as_ref()?
        .iter()
        .find(|found| found.backend == backend)
}

/// Nothing is installed or ready at all: the picker suggests one.
pub(in crate::rune_ui) fn nothing_installed(ui: &Ui) -> bool {
    ui.chats.settings.found.as_ref().is_some_and(|found| {
        found
            .iter()
            .all(|found| !found.ready && found.path.is_none())
    })
}

/// The suggested agent when nothing is installed.
pub(in crate::rune_ui) fn recommended(ui: &Ui) -> AssistantBackend {
    let _ = ui;
    backends::recommendations()
        .first()
        .map_or(AssistantBackend::ClaudeCode, |pick| pick.backend)
}

/// The agent's one action for its state: OpenCode and the Claude API take
/// a key here; the others install, sign in, and check, in one go.
pub(in crate::rune_ui) fn act(ui: &mut Ui, backend: AssistantBackend) {
    ui.update(|settings| settings.assistant.backend = backend);
    match backend {
        AssistantBackend::ClaudeApi => ui.chats.settings.agents.api_key_open = true,
        AssistantBackend::OpenCode => open_byok(ui),
        _ => set_up(ui, backend),
    }
}

/// Installs (or starts, or signs in) `backend`, then checks it answers.
pub(in crate::rune_ui) fn set_up(ui: &mut Ui, backend: AssistantBackend) {
    let settings = ui.settings.assistant.clone();
    let agents = &mut ui.chats.settings.agents;
    if agents.working.is_some() {
        return;
    }
    agents.working = Some(backend);
    agents.log.clear();
    agents.failed = None;
    agents.command = None;
    ui.chats
        .jobs
        .run(Job::SetUpAgent(backend, Box::new(settings)));
}

/// Opens the own-key panel for OpenCode, on DeepSeek.
pub(in crate::rune_ui) fn open_byok(ui: &mut Ui) {
    let agents = &mut ui.chats.settings.agents;
    agents.show_all = true;
    if agents.byok.is_none() {
        agents.byok = Some(Byok::default());
    }
}

/// The provider chosen in the panel.
pub(in crate::rune_ui) fn provider(byok: &Byok) -> Provider {
    let providers = backends::providers();
    providers
        .get(byok.provider)
        .cloned()
        .unwrap_or(providers[0])
}

/// Checks the key, installing OpenCode first when it isn't; on success,
/// saves the key and makes OpenCode on that model the agent.
pub(in crate::rune_ui) fn verify(ui: &mut Ui) {
    let install = found(ui, AssistantBackend::OpenCode).is_none_or(|found| found.path.is_none());
    let Some(byok) = ui.chats.settings.agents.byok.as_mut() else {
        return;
    };
    let key = byok.key.text.trim().to_owned();
    if key.is_empty() || byok.verifying {
        return;
    }
    let provider = provider(byok);
    let model = provider
        .models
        .get(byok.model)
        .copied()
        .unwrap_or(provider.default_model);
    byok.verifying = true;
    byok.result = None;
    if install {
        ui.chats.settings.agents.working = Some(AssistantBackend::OpenCode);
        ui.chats.settings.agents.log.clear();
    }
    ui.chats.jobs.run(Job::OwnKey {
        provider: provider.id.into(),
        key,
        model: model.into(),
        install,
    });
}

/// Saves Codex's OpenAI key in place of a sign-in.
pub(in crate::rune_ui) fn use_codex_key(ui: &mut Ui) {
    let Some(field) = ui.chats.settings.agents.codex_key.as_ref() else {
        return;
    };
    let key = field.text.trim().to_owned();
    if !key.is_empty() {
        ui.chats.jobs.run(Job::CodexKey(key));
    }
}

/// Does a picker job off the window's thread. True when it was one.
pub(in crate::rune_ui) fn perform(job: &Job, send: &dyn Fn(Done)) -> bool {
    match job {
        Job::SetUpAgent(backend, settings) => {
            let result = set_up_now(*backend, settings, send);
            if result.is_err() {
                send(Done::AgentCommand(backends::install_command(Fix::Install(
                    *backend,
                ))));
            }
            send(Done::AgentSetUp(*backend, result));
            backends::refresh();
            send(Done::Detected(backends::detect()));
        }
        Job::OwnKey {
            provider,
            key,
            model,
            install,
        } => {
            let result = own_key_now(provider, key, model, *install, send);
            send(Done::OwnKey(result.map(|()| format!("{provider}/{model}"))));
            backends::refresh();
            send(Done::Detected(backends::detect()));
        }
        Job::CodexKey(key) => {
            // The key is saved as OpenAI's, then Codex signs in with it.
            let result = backends::set_provider_key("openai", key)
                .and_then(|()| {
                    backends::apply_fix_with(Fix::UseApiKey(AssistantBackend::Codex), |line| {
                        send(Done::AgentProgress(
                            AssistantBackend::Codex,
                            line.to_owned(),
                        ))
                    })
                })
                .map_err(|error| format!("{error:#}"));
            send(Done::CodexKey(result));
            backends::refresh();
            send(Done::Detected(backends::detect()));
        }
        _ => return false,
    }
    true
}

fn set_up_now(
    backend: AssistantBackend,
    settings: &AssistantSettings,
    send: &dyn Fn(Done),
) -> Result<(), String> {
    let progress = |line: &str| send(Done::AgentProgress(backend, line.to_owned()));
    // Install, then start or sign in, as the agent needs: at most a few
    // steps, each looked at afresh.
    for _ in 0..3 {
        backends::refresh();
        let found = backends::detect()
            .into_iter()
            .find(|found| found.backend == backend);
        let Some(fix) = found.as_ref().and_then(super::setup::fix_for) else {
            break;
        };
        if matches!(fix, Fix::AddApiKey | Fix::UseApiKey(_)) {
            return Err("Add an API key for this agent".into());
        }
        backends::apply_fix_with(fix, progress).map_err(|error| format!("{error:#}"))?;
    }
    progress("Checking that it answers…");
    let checked = backends::self_test(Check::Answers(backend), settings);
    if checked.ok() {
        Ok(())
    } else {
        Err(checked.detail)
    }
}

fn own_key_now(
    provider: &str,
    key: &str,
    model: &str,
    install: bool,
    send: &dyn Fn(Done),
) -> Result<(), String> {
    let progress = |line: &str| {
        send(Done::AgentProgress(
            AssistantBackend::OpenCode,
            line.to_owned(),
        ))
    };
    if install {
        backends::apply_fix_with(Fix::Install(AssistantBackend::OpenCode), progress)
            .map_err(|error| format!("{error:#}"))?;
    }
    backends::set_provider_key(provider, key).map_err(|error| format!("{error:#}"))?;
    match backends::verify_provider_key(provider) {
        Ok(found) => progress(&found),
        Err(error) => {
            // A key that doesn't work isn't kept.
            let _ = backends::set_provider_key(provider, "");
            return Err(format!("{error:#}"));
        }
    }
    // The key works; now prove OpenCode answers with it, as a task runs it
    // (an OpenCode that can't run tasks would otherwise only show up on the
    // first one). The key stays: it is good.
    progress("Checking that OpenCode answers…");
    let settings = AssistantSettings {
        backend: AssistantBackend::OpenCode,
        opencode_provider: Some(provider.to_owned()),
        opencode_model: Some(format!("{provider}/{model}")),
        ..crate::app_settings::assistant_runtime()
            .map(|(_, settings)| settings)
            .unwrap_or_default()
    };
    let checked = backends::self_test(Check::Answers(AssistantBackend::OpenCode), &settings);
    if checked.ok() {
        Ok(())
    } else {
        Err(checked.detail)
    }
}

/// A picker job's answer. True when it was one.
pub(in crate::rune_ui) fn receive(ui: &mut Ui, done: &Done) -> bool {
    let agents = &mut ui.chats.settings.agents;
    match done {
        Done::AgentProgress(backend, line) => {
            if agents.working == Some(*backend) && !line.trim().is_empty() {
                agents.log.push(line.clone());
                let extra = agents.log.len().saturating_sub(LOG);
                agents.log.drain(..extra);
            }
        }
        Done::AgentCommand(command) => agents.command = command.clone(),
        Done::AgentSetUp(backend, result) => {
            agents.working = None;
            agents.failed = result.clone().err().map(|why| {
                let mut tail = std::mem::take(&mut agents.log);
                tail.push(why);
                (*backend, tail.join("\n"))
            });
        }
        Done::OwnKey(result) => {
            agents.working = None;
            agents.log.clear();
            if let Some(byok) = agents.byok.as_mut() {
                byok.verifying = false;
                byok.result = Some(result.clone());
                if result.is_ok() {
                    byok.key.clear();
                }
            }
            if let Ok(model) = result {
                let model = model.clone();
                let provider = model.split('/').next().unwrap_or_default().to_owned();
                ui.update(|settings| {
                    settings.assistant.backend = AssistantBackend::OpenCode;
                    settings.assistant.opencode_provider = Some(provider);
                    settings.assistant.opencode_model = Some(model);
                });
            }
        }
        Done::CodexKey(result) => {
            agents.codex_error = result.clone().err();
            if result.is_ok() {
                agents.codex_key = None;
            }
        }
        _ => return false,
    }
    true
}

/// Every agent in order, for the list.
pub(in crate::rune_ui) fn all() -> [AssistantBackend; 4] {
    BACKENDS
}
