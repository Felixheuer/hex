//! OpenCode with the person's own API key ("bring your own key"): a
//! catalogue of providers OpenCode knows (ids, key variables, and models as
//! OpenCode 2.2.6 and models.dev list them in November 2026), their keys in
//! the Keychain ([`keys`](super::keys)), a tiny real request that proves a
//! key works, and the OpenCode config fragment and environment that hand
//! the key to a task without writing it anywhere.
//!
//! The fragment names the key by reference (`{env:DEEPSEEK_API_KEY}`, as
//! OpenCode's config substitutes it), and only the process that runs the
//! task gets the variable.

use std::collections::BTreeMap;
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use serde_json::{Value, json};

use super::keys;
use crate::app_settings::{AssistantBackend, AssistantSettings};

/// How a provider takes its key on a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Auth {
    Bearer,
    /// Anthropic's `x-api-key` with `anthropic-version`.
    Anthropic,
    /// Google's `x-goog-api-key`.
    Google,
}

/// One provider OpenCode can use with a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Provider {
    /// OpenCode's provider id (`deepseek`), the part before `/` in a model.
    pub id: &'static str,
    pub name: &'static str,
    /// The page where a person creates a key.
    pub key_url: &'static str,
    /// The model (without the provider) a new setup uses.
    pub default_model: &'static str,
    /// Models offered first (all tool-capable), the default first.
    pub models: &'static [&'static str],
    /// The quick one of them for quick turns (`None`: the default).
    pub fast_model: Option<&'static str>,
    /// The variable OpenCode reads the key from.
    pub env: &'static str,
    /// The API to verify a key against, and the path asked.
    api: &'static str,
    check_path: &'static str,
    auth: Auth,
}

const PROVIDERS: &[Provider] = &[
    Provider {
        id: "deepseek",
        name: "DeepSeek",
        key_url: "https://platform.deepseek.com/api_keys",
        default_model: "deepseek-v4-pro",
        models: &["deepseek-v4-pro", "deepseek-flash"],
        fast_model: Some("deepseek-flash"),
        env: "DEEPSEEK_API_KEY",
        api: "https://api.deepseek.com",
        // Also says whether the account has credit.
        check_path: "/user/balance",
        auth: Auth::Bearer,
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        key_url: "https://openrouter.ai/settings/keys",
        default_model: "anthropic/claude-sonnet-5.5",
        models: &[
            "anthropic/claude-sonnet-5.5",
            "openai/gpt-6.1-sol",
            "google/gemini-4-carbon",
            "deepseek/deepseek-v4-pro",
            "moonshotai/kimi-k3.1-pro",
            "z-ai/glm-5.3",
        ],
        fast_model: None,
        env: "OPENROUTER_API_KEY",
        api: "https://openrouter.ai/api/v1",
        // The key itself: its limit and what's left.
        check_path: "/key",
        auth: Auth::Bearer,
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        key_url: "https://platform.openai.com/api-keys",
        default_model: "gpt-6.1-sol",
        models: &["gpt-6.1-sol", "gpt-6.1-terra", "gpt-6-luna"],
        fast_model: None,
        env: "OPENAI_API_KEY",
        api: "https://api.openai.com/v1",
        check_path: "/models",
        auth: Auth::Bearer,
    },
    Provider {
        id: "anthropic",
        name: "Anthropic",
        key_url: "https://platform.claude.com/settings/keys",
        default_model: "claude-sonnet-5-5",
        models: &["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"],
        fast_model: Some("claude-haiku-5-5"),
        env: "ANTHROPIC_API_KEY",
        api: "https://api.anthropic.com/v1",
        check_path: "/models",
        auth: Auth::Anthropic,
    },
    Provider {
        id: "google",
        name: "Google Gemini",
        key_url: "https://aistudio.google.com/apikey",
        default_model: "gemini-4-carbon",
        models: &["gemini-4-carbon", "gemini-4-argon", "gemini-3.8-flash"],
        fast_model: Some("gemini-3.8-flash"),
        env: "GOOGLE_GENERATIVE_AI_API_KEY",
        api: "https://generativelanguage.googleapis.com/v1beta",
        check_path: "/models",
        auth: Auth::Google,
    },
    Provider {
        id: "groq",
        name: "Groq",
        key_url: "https://console.groq.com/keys",
        default_model: "qwen/qwen3.8-27b",
        models: &["qwen/qwen3.8-27b", "openai/gpt-oss-120b"],
        fast_model: None,
        env: "GROQ_API_KEY",
        api: "https://api.groq.com/openai/v1",
        check_path: "/models",
        auth: Auth::Bearer,
    },
    Provider {
        id: "mistral",
        name: "Mistral",
        key_url: "https://console.mistral.ai/api-keys",
        default_model: "mistral-large-4",
        models: &["mistral-large-4"],
        fast_model: None,
        env: "MISTRAL_API_KEY",
        api: "https://api.mistral.ai/v1",
        check_path: "/models",
        auth: Auth::Bearer,
    },
    Provider {
        id: "xai",
        name: "xAI",
        key_url: "https://console.x.ai/team/default/api-keys",
        default_model: "grok-4.7",
        models: &["grok-4.7", "grok-4.6"],
        fast_model: None,
        env: "XAI_API_KEY",
        api: "https://api.x.ai/v1",
        check_path: "/models",
        auth: Auth::Bearer,
    },
    Provider {
        id: "moonshotai",
        name: "Moonshot (Kimi)",
        key_url: "https://platform.kimi.ai/console/api-keys",
        default_model: "kimi-k3.1-pro",
        models: &["kimi-k3.1-pro", "kimi-k3.1-pro-highspeed", "kimi-k3"],
        fast_model: Some("kimi-k3.1-pro-highspeed"),
        env: "MOONSHOT_API_KEY",
        api: "https://api.moonshot.ai/v1",
        check_path: "/models",
        auth: Auth::Bearer,
    },
    Provider {
        id: "zai",
        name: "Z.ai (GLM)",
        key_url: "https://z.ai/manage-apikey/apikey-list",
        default_model: "glm-5.3",
        models: &["glm-5.3", "glm-5.3-flash"],
        fast_model: Some("glm-5.3-flash"),
        env: "ZHIPU_API_KEY",
        api: "https://api.z.ai/api/paas/v4",
        check_path: "/models",
        auth: Auth::Bearer,
    },
];

/// Every provider Agent-Desk offers for OpenCode, in the order setup lists them.
pub fn providers() -> Vec<Provider> {
    PROVIDERS.to_vec()
}

pub fn provider(id: &str) -> Option<Provider> {
    PROVIDERS.iter().find(|provider| provider.id == id).copied()
}

/// Saves `key` for `provider` in the Keychain; an empty key forgets it.
pub fn set_provider_key(id: &str, key: &str) -> Result<()> {
    let provider =
        provider(id).ok_or_else(|| eyre!("Agent-Desk doesn't know the provider “{id}”"))?;
    keys::save(provider.id, key)
}

/// Whether a key for `id` is saved.
pub fn has_provider_key(id: &str) -> bool {
    provider_key(id).is_some()
}

pub(super) fn provider_key(id: &str) -> Option<String> {
    keys::load(provider(id)?.id)
}

/// Asks the provider, with the saved key, one tiny question (its models, or
/// for DeepSeek and OpenRouter the account's credit). `Ok` says what it
/// found, for a person; `Err` says whether the key is wrong or the account
/// is out of credit. Blocks for seconds at most; off the UI thread.
pub fn verify_provider_key(id: &str) -> Result<String> {
    let provider =
        provider(id).ok_or_else(|| eyre!("Agent-Desk doesn't know the provider “{id}”"))?;
    let key = keys::load(provider.id).ok_or_else(|| eyre!("No {} key is saved", provider.name))?;
    verify(provider, &key, provider.api)
}

/// [`verify_provider_key`] against `api` (a fake server in tests).
fn verify(provider: Provider, key: &str, api: &str) -> Result<String> {
    let url = format!("{api}{}", provider.check_path);
    let agent = crate::net::agent(&url, |config| {
        config
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
    });
    let request = agent.get(&url).header("accept", "application/json");
    let request = match provider.auth {
        Auth::Bearer => request.header("authorization", format!("Bearer {key}")),
        Auth::Anthropic => request
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01"),
        Auth::Google => request.header("x-goog-api-key", key),
    };
    let mut response = request
        .call()
        .map_err(|error| eyre!("{}", crate::net::unreachable(&url, &error)))?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    judge(provider, status, &body).map_err(|why| eyre!("{why}"))
}

/// What a provider's answer to the check means.
pub fn judge(provider: Provider, status: u16, body: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let message = error_message(&value).unwrap_or_else(|| body.trim().chars().take(160).collect());
    let lower = message.to_lowercase();
    let name = provider.name;
    let wrong_key = format!(
        "{name} didn't accept the key: it's wrong or was revoked. Copy it again from {}",
        provider.key_url
    );
    let no_credit = format!(
        "{name} accepted the key, but the account has no credit left. Add some at {}",
        provider.key_url
    );
    match status {
        200 => {}
        401 | 403 => return Err(wrong_key),
        400 if lower.contains("api key") || lower.contains("api_key") => return Err(wrong_key),
        402 => return Err(no_credit),
        429 if lower.contains("quota") || lower.contains("credit") || lower.contains("balance") => {
            return Err(no_credit);
        }
        429 => {
            return Err(format!(
                "{name} is limiting requests right now; try again in a minute"
            ));
        }
        _ => return Err(format!("{name} answered {status}: {message}")),
    }
    match provider.id {
        "deepseek" => {
            if value.get("is_available").and_then(Value::as_bool) == Some(false) {
                return Err(no_credit);
            }
            let balance = value["balance_infos"]
                .as_array()
                .and_then(|infos| infos.first())
                .and_then(|info| {
                    Some(format!(
                        "{} {}",
                        info.get("total_balance")?.as_str()?,
                        info.get("currency")?.as_str()?
                    ))
                });
            Ok(match balance {
                Some(balance) => format!("The key works; the balance is {balance}"),
                None => "The key works".into(),
            })
        }
        "openrouter" => {
            let left = value["data"].get("limit_remaining").and_then(Value::as_f64);
            match left {
                Some(left) if left <= 0.0 => Err(no_credit),
                Some(left) => Ok(format!("The key works; {left:.2} credits are left on it")),
                None => Ok("The key works".into()),
            }
        }
        _ => {
            let count = value
                .get("data")
                .or_else(|| value.get("models"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Ok(if count > 0 {
                format!("The key works; {name} offers {count} models")
            } else {
                "The key works".into()
            })
        }
    }
}

/// The error text in an API's error body, in each provider's shape.
fn error_message(value: &Value) -> Option<String> {
    let error = value.get("error");
    error
        .and_then(|error| error.get("message"))
        .or_else(|| error.filter(|error| error.is_string()))
        .or_else(|| value.get("detail"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `opencode models`' lines (`provider/model`, a model may hold more `/`),
/// by provider.
pub fn parse_models(output: &str) -> BTreeMap<String, Vec<String>> {
    let mut models: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in output.lines().map(str::trim) {
        let Some((provider, model)) = line.split_once('/') else {
            continue;
        };
        let id_like = |text: &str| {
            !text.is_empty()
                && text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.:/@".contains(c))
        };
        if id_like(provider) && !provider.contains('/') && id_like(model) {
            models
                .entry(provider.to_owned())
                .or_default()
                .push(model.to_owned());
        }
    }
    models
}

/// The providers OpenCode has credentials for (its own sign-ins or keys in
/// the environment), from `opencode models`; its own free `opencode`
/// models don't count.
pub fn signed_in_providers(output: &str) -> Vec<String> {
    parse_models(output)
        .into_keys()
        .filter(|provider| provider != "opencode")
        .collect()
}

/// The `provider/model` a task uses: the chosen provider's (the saved
/// model when it is that provider's, else its default), else the saved
/// model.
pub fn opencode_model(settings: &AssistantSettings) -> Option<String> {
    let Some(provider) = settings.opencode_provider.as_deref().and_then(provider) else {
        return settings.opencode_model.clone();
    };
    let prefix = format!("{}/", provider.id);
    Some(
        settings
            .opencode_model
            .clone()
            .filter(|model| model.starts_with(&prefix))
            .unwrap_or_else(|| format!("{prefix}{}", provider.default_model)),
    )
}

/// The `provider/model` a quick turn uses on OpenCode: the quick-question
/// model when it is one of the task's provider's (`assistant.here_model`;
/// its default is a Claude Code model, which OpenCode doesn't take), else
/// that provider's fast model ([`Provider::fast_model`]), else the jobs'.
pub fn opencode_here_model(settings: &AssistantSettings) -> Option<String> {
    let strong = opencode_model(settings);
    let provider_id = strong
        .as_deref()
        .and_then(|model| model.split_once('/'))
        .map(|(provider, _)| provider.to_owned());
    if let Some(here) = settings
        .here_model
        .as_deref()
        .map(str::trim)
        .filter(|model| model.contains('/'))
        && provider_id
            .as_deref()
            .is_none_or(|id| here.starts_with(&format!("{id}/")))
    {
        return Some(here.to_owned());
    }
    provider_id
        .as_deref()
        .and_then(provider)
        .and_then(|provider| {
            provider
                .fast_model
                .map(|fast| format!("{}/{fast}", provider.id))
        })
        .or(strong)
}

/// The OpenCode config fragment for the chosen provider: its model, and
/// its key by reference to the variable [`agent_env_for`] sets. `None`
/// without a chosen provider. Merge it into `OPENCODE_CONFIG_CONTENT`.
pub fn opencode_provider_config(settings: &AssistantSettings) -> Option<Value> {
    let provider = provider(settings.opencode_provider.as_deref()?)?;
    Some(json!({
        "model": opencode_model(settings)?,
        "providers": {
            provider.id: { "settings": { "apiKey": format!("{{env:{}}}", provider.env) } }
        },
    }))
}

/// [`super::agent_env`] for a task of `backend`, plus the chosen
/// provider's key for OpenCode. Use it whole.
pub fn agent_env_for(
    backend: AssistantBackend,
    settings: &AssistantSettings,
) -> Vec<(String, String)> {
    with_key(super::agent_env(), backend, settings, provider_key)
}

fn with_key(
    mut env: Vec<(String, String)>,
    backend: AssistantBackend,
    settings: &AssistantSettings,
    key: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let chosen = settings.opencode_provider.as_deref().and_then(provider);
    if backend == AssistantBackend::OpenCode
        && let Some(provider) = chosen
        && let Some(key) = key(provider.id)
    {
        env.retain(|(name, _)| name != provider.env);
        env.push((provider.env.to_owned(), key));
    }
    env
}

/// `base` (an `OPENCODE_CONFIG_CONTENT` object) with `fragment` merged in,
/// objects key by key, the fragment winning.
pub fn merge_config(base: &mut Value, fragment: &Value) {
    match (base, fragment) {
        (Value::Object(base), Value::Object(fragment)) => {
            for (key, value) in fragment {
                match base.get_mut(key) {
                    Some(existing) if existing.is_object() && value.is_object() => {
                        merge_config(existing, value);
                    }
                    _ => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, fragment) => *base = fragment.clone(),
    }
}

/// An `opencode run` for a self-test, as tasks run it: its `run`
/// arguments for the installed OpenCode, the chosen provider's config
/// merged into `config` and its key in the environment. Returns the model
/// to pass; the caller adds it and the prompt.
pub(super) fn opencode_check(
    command: &mut std::process::Command,
    settings: &AssistantSettings,
    mut config: Value,
) -> Option<String> {
    let generation = super::opencode::generation();
    if let Some(fragment) = opencode_provider_config(settings) {
        merge_config(&mut config, &fragment);
    }
    super::opencode::adapt_config(&mut config, generation);
    command.args(super::opencode::run_args(generation));
    if config.as_object().is_some_and(|config| !config.is_empty()) {
        command.env("OPENCODE_CONFIG_CONTENT", config.to_string());
    }
    let env = with_key(
        Vec::new(),
        AssistantBackend::OpenCode,
        settings,
        provider_key,
    );
    command.envs(env);
    opencode_model(settings).or_else(super::opencode_recent_model)
}

/// [`Fix::UseApiKey`](super::Fix::UseApiKey): the agent uses a saved key
/// instead of an account. Codex is signed in with the OpenAI key (`codex
/// login --with-api-key`, the key on its stdin), which replaces its ChatGPT
/// sign-in everywhere; OpenCode and the Claude API only need the key saved.
pub(super) fn use_api_key(backend: AssistantBackend, progress: &mut dyn FnMut(&str)) -> Result<()> {
    match backend {
        AssistantBackend::Codex => {
            let key = keys::load("openai")
                .ok_or_else(|| eyre!("Add an OpenAI API key first, then try again"))?;
            let path = super::executable(backend).ok_or_else(|| eyre!("Codex isn't installed"))?;
            progress("Signing Codex in with your OpenAI API key…");
            codex_login_with_key(&path, &key)?;
            let (ready, note) = super::codex_signed_in(&path);
            if !ready {
                return Err(eyre!(
                    "Codex didn't take the key: {}",
                    note.unwrap_or_default()
                ));
            }
            progress("Codex uses your OpenAI API key");
            Ok(())
        }
        AssistantBackend::OpenCode => {
            let chosen = crate::app_settings::assistant_runtime()
                .and_then(|(_, settings)| settings.opencode_provider);
            if byok_ready(chosen.as_deref()) {
                let name = chosen
                    .as_deref()
                    .and_then(provider)
                    .map_or("your", |p| p.name);
                progress(&format!("OpenCode uses {name} API key"));
                Ok(())
            } else {
                Err(eyre!("Choose a provider and add its API key for OpenCode"))
            }
        }
        AssistantBackend::ClaudeApi => {
            if crate::assistant::credentials::has_api_key() {
                Ok(())
            } else {
                Err(eyre!("Add an Anthropic API key"))
            }
        }
        AssistantBackend::ClaudeCode => Err(eyre!(
            "Claude Code signs in with a Claude account. To use an Anthropic API key, choose \
             the Claude API instead"
        )),
    }
}

/// `codex login --with-api-key`, the key written to its stdin.
fn codex_login_with_key(path: &std::path::Path, key: &str) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;
    let mut command = super::agent_command(path);
    command
        .args(["login", "--with-api-key"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::run::own_group(&mut command, true);
    let mut child = command
        .spawn()
        .map_err(|error| eyre!("Codex didn't start: {error}"))?;
    let tree = super::run::Tree::of(&child);
    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{key}")?;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()? {
            let output = child.wait_with_output()?;
            if status.success() {
                return Ok(());
            }
            let said = String::from_utf8_lossy(&output.stderr);
            let last = said.lines().map(str::trim).rfind(|line| !line.is_empty());
            return Err(eyre!(
                "Codex didn't sign in with the key: {}",
                last.unwrap_or("it gave no reason")
            ));
        }
        if std::time::Instant::now() >= deadline {
            tree.kill_with(&mut child);
            return Err(eyre!("Codex took too long to sign in with the key"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Whether a BYOK key is there for OpenCode: the chosen provider's, or
/// (with none chosen) any provider's.
pub(super) fn byok_ready(chosen: Option<&str>) -> bool {
    match chosen.and_then(provider) {
        Some(provider) => keys::load(provider.id).is_some(),
        None => PROVIDERS
            .iter()
            .any(|provider| keys::load(provider.id).is_some()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    const RECORDED: &str =
        include_str!("../../../tests/fixtures/assistant/opencode-models-2.2.6.txt");

    #[test]
    fn this_macs_opencode_models_are_read_by_provider() {
        let models = parse_models(RECORDED);
        assert_eq!(
            models.keys().collect::<Vec<_>>(),
            ["deepseek", "openai", "opencode"]
        );
        assert_eq!(models["deepseek"], ["deepseek-flash", "deepseek-v4-pro"]);
        assert!(models["openai"].contains(&"gpt-6.1-sol".to_owned()));
        assert_eq!(signed_in_providers(RECORDED), ["deepseek", "openai"]);
        // Every DeepSeek and OpenAI model in the catalogue is one this
        // OpenCode lists.
        for id in ["deepseek", "openai"] {
            for model in provider(id).unwrap().models {
                assert!(models[id].contains(&(*model).to_owned()), "{id}/{model}");
            }
        }
        // A model may hold more `/`; noise lines are skipped.
        let output =
            "openrouter/anthropic/claude-sonnet-5.5\nWarning: something\n\nopencode/big-pickle\n";
        let models = parse_models(output);
        assert_eq!(models["openrouter"], ["anthropic/claude-sonnet-5.5"]);
        assert_eq!(signed_in_providers(output), ["openrouter"]);
        assert!(signed_in_providers("opencode/big-pickle\n").is_empty());
    }

    #[test]
    fn the_catalogue_is_complete_and_consistent() {
        let ids: Vec<_> = providers().iter().map(|provider| provider.id).collect();
        assert_eq!(
            ids,
            [
                "deepseek",
                "openrouter",
                "openai",
                "anthropic",
                "google",
                "groq",
                "mistral",
                "xai",
                "moonshotai",
                "zai"
            ]
        );
        for provider in providers() {
            assert_eq!(
                provider.models[0], provider.default_model,
                "{}",
                provider.id
            );
            assert!(provider.key_url.starts_with("https://"));
            assert!(provider.env.ends_with("_API_KEY"));
            if let Some(fast) = provider.fast_model {
                assert!(provider.models.contains(&fast), "{}", provider.id);
            }
        }
    }

    #[test]
    fn quick_turns_take_the_providers_fast_model() {
        let with = |provider: Option<&str>, model: Option<&str>, here: Option<&str>| {
            opencode_here_model(&AssistantSettings {
                here_model: here.map(str::to_owned),
                ..settings(provider, model)
            })
        };
        // The default quick-question model is Claude Code's: not OpenCode's.
        assert_eq!(
            with(Some("deepseek"), None, Some("claude-sonnet-5-5")).as_deref(),
            Some("deepseek/deepseek-flash")
        );
        // The person's own sign-in: the saved model's provider.
        assert_eq!(
            with(None, Some("deepseek/deepseek-v4-pro"), None).as_deref(),
            Some("deepseek/deepseek-flash")
        );
        // A quick-question model of that provider wins; another's doesn't.
        assert_eq!(
            with(Some("deepseek"), None, Some("deepseek/deepseek-v4-pro")).as_deref(),
            Some("deepseek/deepseek-v4-pro")
        );
        assert_eq!(
            with(Some("deepseek"), None, Some("openai/gpt-6-luna")).as_deref(),
            Some("deepseek/deepseek-flash")
        );
        // No fast model known: the jobs' one.
        assert_eq!(
            with(Some("openai"), None, None).as_deref(),
            Some("openai/gpt-6.1-sol")
        );
        assert_eq!(
            with(None, Some("opencode/big-pickle"), None).as_deref(),
            Some("opencode/big-pickle")
        );
        assert_eq!(with(None, None, None), None);
    }

    fn settings(provider: Option<&str>, model: Option<&str>) -> AssistantSettings {
        AssistantSettings {
            opencode_provider: provider.map(str::to_owned),
            opencode_model: model.map(str::to_owned),
            ..AssistantSettings::default()
        }
    }

    #[test]
    fn the_config_names_the_key_by_reference_and_picks_the_model() {
        let deepseek = settings(Some("deepseek"), None);
        assert_eq!(
            opencode_provider_config(&deepseek),
            Some(json!({
                "model": "deepseek/deepseek-v4-pro",
                "providers": {"deepseek": {"settings": {"apiKey": "{env:DEEPSEEK_API_KEY}"}}},
            }))
        );
        // A saved model of that provider wins; one of another doesn't.
        assert_eq!(
            opencode_model(&settings(Some("deepseek"), Some("deepseek/deepseek-flash"))).as_deref(),
            Some("deepseek/deepseek-flash")
        );
        assert_eq!(
            opencode_model(&settings(
                Some("openrouter"),
                Some("deepseek/deepseek-flash")
            ))
            .as_deref(),
            Some("openrouter/anthropic/claude-sonnet-5.5")
        );
        assert_eq!(
            opencode_model(&settings(None, Some("opencode/big-pickle"))).as_deref(),
            Some("opencode/big-pickle")
        );
        assert_eq!(opencode_provider_config(&settings(None, None)), None);
        assert_eq!(
            opencode_provider_config(&settings(Some("nope"), None)),
            None
        );
        let mut base = json!({"agent": {"hex": {"prompt": "x"}}, "mcp": {"a": {}}});
        merge_config(&mut base, &opencode_provider_config(&deepseek).unwrap());
        assert_eq!(base["agent"]["hex"]["prompt"], "x");
        assert_eq!(base["model"], "deepseek/deepseek-v4-pro");
        assert!(base["providers"]["deepseek"].is_object());
    }

    #[test]
    fn only_opencode_tasks_get_the_key_in_their_environment() {
        let chosen = settings(Some("groq"), None);
        let base = vec![
            ("PATH".to_owned(), "/usr/bin".to_owned()),
            ("GROQ_API_KEY".to_owned(), "stale".to_owned()),
        ];
        let key = |id: &str| (id == "groq").then(|| "gsk-test".to_owned());
        let env = with_key(base.clone(), AssistantBackend::OpenCode, &chosen, key);
        assert_eq!(
            env,
            [
                ("PATH".to_owned(), "/usr/bin".to_owned()),
                ("GROQ_API_KEY".to_owned(), "gsk-test".to_owned())
            ]
        );
        assert_eq!(
            with_key(base.clone(), AssistantBackend::Codex, &chosen, key),
            base
        );
        assert_eq!(
            with_key(base.clone(), AssistantBackend::OpenCode, &chosen, |_| None),
            base
        );
    }

    #[test]
    fn answers_to_the_check_are_judged() {
        let deepseek = provider("deepseek").unwrap();
        let rich =
            r#"{"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"4.20"}]}"#;
        assert_eq!(
            judge(deepseek, 200, rich).unwrap(),
            "The key works; the balance is 4.20 USD"
        );
        let broke =
            r#"{"is_available":false,"balance_infos":[{"currency":"USD","total_balance":"0.00"}]}"#;
        assert!(
            judge(deepseek, 200, broke)
                .unwrap_err()
                .contains("no credit")
        );
        let wrong = r#"{"error":{"message":"Authentication Fails","type":"authentication_error"}}"#;
        assert!(
            judge(deepseek, 401, wrong)
                .unwrap_err()
                .contains("didn't accept the key")
        );
        let openrouter = provider("openrouter").unwrap();
        assert!(
            judge(openrouter, 200, r#"{"data":{"limit_remaining":0}}"#)
                .unwrap_err()
                .contains("no credit")
        );
        let google = provider("google").unwrap();
        let invalid = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}"#;
        assert!(
            judge(google, 400, invalid)
                .unwrap_err()
                .contains("didn't accept")
        );
        let groq = provider("groq").unwrap();
        assert_eq!(
            judge(
                groq,
                200,
                r#"{"object":"list","data":[{"id":"a"},{"id":"b"}]}"#
            )
            .unwrap(),
            "The key works; Groq offers 2 models"
        );
        assert!(
            judge(
                groq,
                429,
                r#"{"error":{"message":"You exceeded your current quota"}}"#
            )
            .unwrap_err()
            .contains("no credit")
        );
        assert!(judge(groq, 402, "{}").unwrap_err().contains("no credit"));
        let mistral = provider("mistral").unwrap();
        assert!(
            judge(mistral, 500, r#"{"detail":"boom"}"#)
                .unwrap_err()
                .ends_with("answered 500: boom")
        );
    }

    /// A one-request HTTP server on loopback that answers `status` and
    /// `body`, and hands back the request it got.
    fn fake_server(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let served = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            while let Ok(read @ 1..) = stream.read(&mut chunk) {
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (address, served)
    }

    #[test]
    fn a_key_is_verified_with_one_real_request() {
        let (api, served) = fake_server(
            200,
            r#"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"12.00"}]}"#,
        );
        let deepseek = provider("deepseek").unwrap();
        assert_eq!(
            verify(deepseek, "sk-test", &api).unwrap(),
            "The key works; the balance is 12.00 CNY"
        );
        let request = served.join().unwrap().to_lowercase();
        assert!(request.starts_with("get /user/balance "), "{request}");
        assert!(
            request.contains("authorization: bearer sk-test"),
            "{request}"
        );

        let (api, served) = fake_server(
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
        );
        let anthropic = provider("anthropic").unwrap();
        let error = verify(anthropic, "sk-ant-test", &api)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Anthropic didn't accept the key"), "{error}");
        let request = served.join().unwrap().to_lowercase();
        assert!(request.contains("x-api-key: sk-ant-test"), "{request}");
        assert!(
            request.contains("anthropic-version: 2023-06-01"),
            "{request}"
        );
    }
}
