//! The Anthropic API key, kept where this computer keeps secrets
//! (`assistant::secrets`: the Keychain, the keyring, or Credential
//! Manager; service "Hex Assistant", account "anthropic"), never in
//! settings.json. The `ANTHROPIC_API_KEY` environment variable wins when
//! set.
//!
//! OWNER: brain.

use color_eyre::{Result, eyre::eyre};

const ACCOUNT: &str = "anthropic";

/// The key to use, if any.
pub fn api_key() -> Option<String> {
    if let Some(key) = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
    {
        return Some(key);
    }
    super::secrets::load(ACCOUNT)
}

/// Whether a key is saved or set in the environment.
pub fn has_api_key() -> bool {
    api_key().is_some()
}

/// Saves `key` (trimmed); an empty key deletes the saved one.
pub fn set_api_key(key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        return super::secrets::delete(ACCOUNT);
    }
    if !super::secrets::plausible(key) {
        return Err(eyre!("that doesn't look like an API key"));
    }
    super::secrets::save(ACCOUNT, key)
}
