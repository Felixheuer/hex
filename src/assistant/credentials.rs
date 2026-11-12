//! The Anthropic API key, kept in the OS's secret store (`store`: the
//! login Keychain on macOS, Credential Manager on Windows, the keyring on
//! Linux; service "Hex Assistant", account "anthropic"), never in
//! settings.json. The `ANTHROPIC_API_KEY` environment variable wins when
//! set.
//!
//! OWNER: brain.

pub mod store;

use color_eyre::Result;

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
    store::load(ACCOUNT)
}

/// Whether a key is saved or set in the environment.
pub fn has_api_key() -> bool {
    api_key().is_some()
}

/// Saves `key` (trimmed); an empty key deletes the saved one.
pub fn set_api_key(key: &str) -> Result<()> {
    store::save(ACCOUNT, key)
}
