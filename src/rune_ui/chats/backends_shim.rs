//! TEMPORARY SHIM: remove at integration.
//!
//! Stand-ins, with the backends engineer's names, for what the agent
//! picker builds on: `backends::recommendations()`, `backends::providers()`,
//! `backends::verify_provider_key`, `backends::set_provider_key`, and
//! `Fix::UseApiKey(Codex)` (here [`use_api_key`]). At integration: delete
//! this file and `mod backends_shim;`, and read `backends_shim::` as
//! `backends::` in `agents.rs`, `agent_picker.rs`, and `jobs.rs`
//! (`use_api_key(b, key)` becomes `apply_fix(Fix::UseApiKey(b))` with the
//! key as that fix expects it).

use color_eyre::{Result, eyre::eyre};

use crate::app_settings::AssistantBackend;
use crate::assistant::backends::Found;

/// The agent to suggest first, and why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) struct Recommendation {
    pub(in crate::rune_ui) backend: AssistantBackend,
    pub(in crate::rune_ui) why: String,
}

/// Best first: Claude Code, unless another agent is already ready.
pub(in crate::rune_ui) fn recommendations(found: &[Found]) -> Vec<Recommendation> {
    let mut picks: Vec<Recommendation> = found
        .iter()
        .filter(|found| found.ready)
        .map(|found| Recommendation {
            backend: found.backend,
            why: "Already set up on this Mac".into(),
        })
        .collect();
    if !picks
        .iter()
        .any(|pick| pick.backend == AssistantBackend::ClaudeCode)
    {
        picks.push(Recommendation {
            backend: AssistantBackend::ClaudeCode,
            why: "Best on the desktop, with your Claude plan".into(),
        });
    }
    picks
}

/// A model provider OpenCode can use with the person's own key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) struct Provider {
    /// OpenCode's provider id (`deepseek`).
    pub(in crate::rune_ui) id: &'static str,
    pub(in crate::rune_ui) name: &'static str,
    /// Where to get a key.
    pub(in crate::rune_ui) key_url: &'static str,
    /// Its models, the default first.
    pub(in crate::rune_ui) models: &'static [&'static str],
}

pub(in crate::rune_ui) fn providers() -> Vec<Provider> {
    let provider = |id, name, key_url, models| Provider {
        id,
        name,
        key_url,
        models,
    };
    vec![
        provider("deepseek", "DeepSeek", "https://platform.deepseek.com/api_keys", &["deepseek-chat", "deepseek-reasoner"]),
        provider("openrouter", "OpenRouter", "https://openrouter.ai/keys", &["anthropic/claude-sonnet-5.5", "openai/gpt-6", "deepseek/deepseek-chat"]),
        provider("openai", "OpenAI", "https://platform.openai.com/api-keys", &["gpt-6", "gpt-6-mini"]),
        provider("anthropic", "Anthropic", "https://console.anthropic.com/settings/keys", &["claude-sonnet-5-5", "claude-opus-5-5"]),
        provider("google", "Gemini", "https://aistudio.google.com/apikey", &["gemini-3-pro", "gemini-3-flash"]),
        provider("groq", "Groq", "https://console.groq.com/keys", &["llama-4-70b"]),
        provider("mistral", "Mistral", "https://console.mistral.ai/api-keys", &["mistral-large-latest"]),
        provider("xai", "xAI", "https://console.x.ai", &["grok-5"]),
        provider("moonshot", "Kimi", "https://platform.moonshot.ai/console/api-keys", &["kimi-k3"]),
        provider("zhipu", "GLM", "https://open.bigmodel.cn/usercenter/apikeys", &["glm-5"]),
    ]
}

/// Asks `provider` whether `key` works with `model`; blocks.
pub(in crate::rune_ui) fn verify_provider_key(provider: &str, key: &str, model: &str) -> Result<()> {
    let _ = (provider, model);
    if key.trim().len() < 8 {
        return Err(eyre!("That doesn't look like a whole key"));
    }
    Err(eyre!("Checking keys isn't in this build of Hex yet"))
}

/// Saves `key` for `provider` where OpenCode reads it (never in settings).
pub(in crate::rune_ui) fn set_provider_key(provider: &str, key: &str) -> Result<()> {
    let _ = (provider, key);
    Err(eyre!("Saving keys isn't in this build of Hex yet"))
}

/// Signs `backend` in with an API key instead of its account (Codex with
/// an OpenAI key).
pub(in crate::rune_ui) fn use_api_key(backend: AssistantBackend, key: &str) -> Result<()> {
    let _ = (backend, key);
    Err(eyre!("Using an API key for this agent isn't in this build of Hex yet"))
}
