//! Provider API keys in the OS's secret store (service "Hex Assistant",
//! account `provider:<id>`; `credentials::store`), never in settings.json.

use color_eyre::Result;

use crate::assistant::credentials::store;

fn account(provider: &str) -> String {
    format!("provider:{provider}")
}

/// Saves `key` (trimmed) for `provider`; an empty key deletes it.
pub fn save(provider: &str, key: &str) -> Result<()> {
    store::save(&account(provider), key)
}

/// `provider`'s saved key.
pub fn load(provider: &str) -> Option<String> {
    store::load(&account(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_kept_per_provider() {
        assert_eq!(account("deepseek"), "provider:deepseek");
    }
}
