//! The Anthropic API key, kept in the login Keychain (service
//! "Hex Assistant", account "anthropic"), never in settings.json. The
//! `ANTHROPIC_API_KEY` environment variable wins when set.
//!
//! The Keychain is reached through `/usr/bin/security`, which every Mac
//! has; the key is passed to it on its standard input, never as an argument.
//!
//! OWNER: brain.

use std::io::Write;
use std::process::{Command, Stdio};

use color_eyre::{Result, eyre::eyre};

const SECURITY: &str = "/usr/bin/security";
const SERVICE: &str = "Hex Assistant";
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
    saved_key()
}

/// Whether a key is saved or set in the environment.
pub fn has_api_key() -> bool {
    api_key().is_some()
}

/// Saves `key` (trimmed); an empty key deletes the saved one.
pub fn set_api_key(key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        return delete_key();
    }
    if key
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '\\')
    {
        return Err(eyre!("that doesn't look like an API key"));
    }
    // `security -i` reads its command from standard input, so the key never
    // shows in the process list.
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
            "add-generic-password -U -s \"{SERVICE}\" -a \"{ACCOUNT}\" -l \"{SERVICE}\" -w \"{key}\""
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

fn saved_key() -> Option<String> {
    let output = Command::new(SECURITY)
        .args(["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
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

fn delete_key() -> Result<()> {
    let output = Command::new(SECURITY)
        .args(["delete-generic-password", "-s", SERVICE, "-a", ACCOUNT])
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
