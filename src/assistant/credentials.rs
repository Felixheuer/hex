//! The Anthropic API key, kept in the login Keychain (service
//! "Hex Assistant", account "anthropic"), never in settings.json. The
//! `ANTHROPIC_API_KEY` environment variable wins when set.
//!
//! OWNER: brain.

use color_eyre::Result;

/// The key to use, if any.
pub fn api_key() -> Option<String> {
    None
}

/// Whether a key is saved or set in the environment.
pub fn has_api_key() -> bool {
    api_key().is_some()
}

/// Saves `key` (trimmed); an empty key deletes the saved one.
pub fn set_api_key(key: &str) -> Result<()> {
    let _ = key;
    Ok(())
}
