//! The OS's own secret store, for the assistant's API keys (never
//! settings.json): the login Keychain on macOS, Credential Manager on
//! Windows, the desktop's keyring (the freedesktop Secret Service: GNOME
//! Keyring, KWallet) on Linux. Every key is one secret under the service
//! "Hex Assistant" and an account of its own ("anthropic",
//! "provider:deepseek"). Without a store Hex says so; it never writes a key
//! in plain text.

#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "linux")]
mod secret_service;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use keychain as imp;
#[cfg(target_os = "linux")]
use secret_service as imp;
#[cfg(windows)]
use windows as imp;

use color_eyre::{Result, eyre::eyre};

use crate::assistant::backends::os::Os;

/// The service every key is saved under.
pub const SERVICE: &str = "Hex Assistant";

/// What the person calls `os`'s secret store, for messages.
pub fn store_name(os: Os) -> &'static str {
    match os {
        Os::Mac => "Keychain",
        Os::Windows => "Credential Manager",
        Os::Linux => "keyring",
    }
}

/// Credential Manager's name for `account`'s key: "Hex Assistant/anthropic".
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "only Credential Manager names keys this way")
)]
pub fn target(account: &str) -> String {
    format!("{SERVICE}/{account}")
}

/// Whether `key` could be an API key: no spaces, control characters,
/// quotes, or backslashes (which also keeps it safe inside `security -i`'s
/// quoted command on macOS).
pub fn plausible(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace() && c != '"' && c != '\\')
}

/// "couldn't open the Keychain: …" for this OS.
pub(super) fn unreachable(os: Os, error: impl std::fmt::Display) -> color_eyre::Report {
    eyre!("couldn't open the {}: {error}", store_name(os))
}

/// "the Keychain didn't save the key: …" for this OS.
pub(super) fn refused(os: Os, what: &str, error: impl std::fmt::Display) -> color_eyre::Report {
    eyre!("the {} didn't {what} the key: {error}", store_name(os))
}

/// Saves `key` (trimmed) for `account`; an empty key deletes it.
pub fn save(account: &str, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        return delete(account);
    }
    if !plausible(key) {
        return Err(eyre!("that doesn't look like an API key"));
    }
    imp::save(account, key)
}

/// `account`'s saved key, if any.
pub fn load(account: &str) -> Option<String> {
    imp::load(account)
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
}

/// Deletes `account`'s saved key; nothing saved is fine.
pub fn delete(account: &str) -> Result<()> {
    imp::delete(account)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_os_names_its_own_store() {
        assert_eq!(store_name(Os::Mac), "Keychain");
        assert_eq!(store_name(Os::Windows), "Credential Manager");
        assert_eq!(store_name(Os::Linux), "keyring");
        assert_eq!(
            refused(Os::Windows, "save", "access denied").to_string(),
            "the Credential Manager didn't save the key: access denied"
        );
        assert_eq!(
            unreachable(Os::Linux, "no bus").to_string(),
            "couldn't open the keyring: no bus"
        );
    }

    #[test]
    fn keys_are_named_per_account_and_checked_before_saving() {
        assert_eq!(target("anthropic"), "Hex Assistant/anthropic");
        assert_eq!(
            target("provider:deepseek"),
            "Hex Assistant/provider:deepseek"
        );
        assert!(plausible("sk-or-v1-abc123"));
        assert!(!plausible("sk abc"));
        assert!(!plausible("sk\"; rm"));
        assert!(!plausible("sk\\n"));
        assert!(!plausible(""));
    }
}
