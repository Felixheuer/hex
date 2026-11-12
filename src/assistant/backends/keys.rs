//! Provider API keys in the login Keychain (service "Hex Assistant",
//! account `provider:<id>`), never in settings.json. Reached through
//! `/usr/bin/security`, with the key on its standard input, never as an
//! argument, so it never shows in the process list.

use std::io::Write;
use std::process::{Command, Stdio};

use color_eyre::{Result, eyre::eyre};

const SECURITY: &str = "/usr/bin/security";
const SERVICE: &str = "Hex Assistant";

fn account(provider: &str) -> String {
    format!("provider:{provider}")
}

/// Whether `key` could be an API key (and is safe inside `security -i`'s
/// quoted command).
pub fn plausible(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace() && c != '"' && c != '\\')
}

/// Saves `key` (trimmed) for `provider`; an empty key deletes it.
pub fn save(provider: &str, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        return delete(provider);
    }
    if !plausible(key) {
        return Err(eyre!("that doesn't look like an API key"));
    }
    let account = account(provider);
    let mut child = Command::new(SECURITY)
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| eyre!("couldn't open the Keychain: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        writeln!(
            stdin,
            "add-generic-password -U -s \"{SERVICE}\" -a \"{account}\" -l \"{SERVICE}\" -w \"{key}\""
        )?;
    }
    let output = child.wait_with_output()?;
    // Interactive mode reports a failed command on stderr, not in its status.
    if output.status.success() && String::from_utf8_lossy(&output.stderr).trim().is_empty() {
        Ok(())
    } else {
        Err(eyre!(
            "the Keychain didn't save the key: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// `provider`'s saved key.
pub fn load(provider: &str) -> Option<String> {
    let output = Command::new(SECURITY)
        .args([
            "find-generic-password",
            "-s",
            SERVICE,
            "-a",
            &account(provider),
            "-w",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let key = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!key.is_empty()).then_some(key)
}

fn delete(provider: &str) -> Result<()> {
    let output = Command::new(SECURITY)
        .args([
            "delete-generic-password",
            "-s",
            SERVICE,
            "-a",
            &account(provider),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map_err(|error| eyre!("couldn't open the Keychain: {error}"))?;
    // 44: errSecItemNotFound, nothing was saved.
    match output.status.code() {
        Some(0) | Some(44) => Ok(()),
        _ => Err(eyre!(
            "the Keychain didn't delete the key: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
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
