//! Provider API keys, kept where this computer keeps secrets
//! (`assistant::secrets`: the Keychain, the keyring, or Credential
//! Manager; service "Agent-Desk Assistant", account `provider:<id>`), never in
//! settings.json.

use color_eyre::{Result, eyre::eyre};

pub use crate::assistant::secrets::plausible;

fn account(provider: &str) -> String {
    format!("provider:{provider}")
}

/// Saves `key` (trimmed) for `provider`; an empty key deletes it.
pub fn save(provider: &str, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        return crate::assistant::secrets::delete(&account(provider));
    }
    if !plausible(key) {
        return Err(eyre!("that doesn't look like an API key"));
    }
    crate::assistant::secrets::save(&account(provider), key)
}

/// `provider`'s saved key.
pub fn load(provider: &str) -> Option<String> {
    crate::assistant::secrets::load(&account(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_kept_per_provider_and_checked_before_saving() {
        assert_eq!(account("deepseek"), "provider:deepseek");
        assert!(plausible("sk-or-v1-abc123"));
        assert!(!plausible("sk abc"));
        assert!(!plausible("sk\"; rm"));
        assert!(!plausible(""));
    }
}
