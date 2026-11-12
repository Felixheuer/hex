//! The login Keychain, reached through `/usr/bin/security`, which every Mac
//! has; the key is passed on its standard input, never as an argument, so
//! it never shows in the process list.

use std::io::Write;
use std::process::{Command, Stdio};

use color_eyre::Result;

use super::{SERVICE, refused, unreachable};
use crate::assistant::backends::os::Os;

const SECURITY: &str = "/usr/bin/security";

/// Saves `key` (already checked to be plausible) for `account`.
pub fn save(account: &str, key: &str) -> Result<()> {
    // `security -i` reads its command from standard input, so the key never
    // shows in the process list.
    let mut child = Command::new(SECURITY)
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| unreachable(Os::Mac, error))?;
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
        Err(refused(
            Os::Mac,
            "save",
            String::from_utf8_lossy(&output.stderr).trim(),
        ))
    }
}

pub fn load(account: &str) -> Option<String> {
    let output = Command::new(SECURITY)
        .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

pub fn delete(account: &str) -> Result<()> {
    let output = Command::new(SECURITY)
        .args(["delete-generic-password", "-s", SERVICE, "-a", account])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map_err(|error| unreachable(Os::Mac, error))?;
    // 44: errSecItemNotFound, nothing was saved.
    match output.status.code() {
        Some(0) | Some(44) => Ok(()),
        _ => Err(refused(
            Os::Mac,
            "delete",
            String::from_utf8_lossy(&output.stderr).trim(),
        )),
    }
}
