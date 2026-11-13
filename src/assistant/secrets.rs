//! Where the assistant's API keys live, never in settings.json: the login
//! Keychain on macOS (`/usr/bin/security`), the Secret Service on Linux
//! (GNOME Keyring, KWallet, KeePassXC… through `secret-tool`, with a
//! private file in Hex's settings folder when no keyring answers, as on a
//! bare i3), and Credential Manager on Windows. Every key is saved under
//! the service "Agent-Desk Assistant" and an account (`anthropic`,
//! `provider:<id>`), and is handed over on standard input or in memory,
//! never as an argument, so it never shows in the process list.
//!
//! OWNER: brain.

use color_eyre::Result;

/// The service every key is saved under.
pub const SERVICE: &str = "Agent-Desk Assistant";

/// What this computer calls the place keys are kept, for messages:
/// "the Keychain", "the keyring", "Credential Manager".
pub fn store_name() -> &'static str {
    system::NAME
}

/// Whether `key` could be an API key: no spaces, controls, quotes, or
/// backslashes (which `security -i`'s quoted command can't hold).
pub fn plausible(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace() && c != '"' && c != '\\')
}

/// Saves `key` for `account`.
pub fn save(account: &str, key: &str) -> Result<()> {
    system::save(SERVICE, account, key)
}

/// `account`'s saved key, if any.
pub fn load(account: &str) -> Option<String> {
    let found = |service| {
        system::load(service, account)
            .map(|key: String| key.trim().to_owned())
            .filter(|key| !key.is_empty())
    };
    found(SERVICE).or_else(|| {
        let key = found(crate::app_paths::legacy::KEYCHAIN_SERVICE)?;
        move_legacy_key(account, &key);
        Some(key)
    })
}

/// One-time migration, remove after 2026-12: a key Hex saved under its old
/// service ("Hex Assistant") moves to [`SERVICE`] the first time it is read.
/// The old item goes only once the new one is saved, so a failed save
/// leaves it where it was (and it is still used this time).
fn move_legacy_key(account: &str, key: &str) {
    match system::save(SERVICE, account, key) {
        Ok(()) => {
            if let Err(error) = system::delete(crate::app_paths::legacy::KEYCHAIN_SERVICE, account) {
                tracing::warn!(%error, "could not remove the key Hex saved");
            }
        }
        Err(error) => tracing::warn!(%error, "could not move the key Hex saved"),
    }
}

/// Deletes `account`'s key, and one Hex saved under its old service; nothing
/// saved is fine.
pub fn delete(account: &str) -> Result<()> {
    system::delete(SERVICE, account)?;
    system::delete(crate::app_paths::legacy::KEYCHAIN_SERVICE, account)
}

#[cfg(target_os = "macos")]
mod system {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use color_eyre::{Result, eyre::eyre};

    pub const NAME: &str = "the Keychain";
    const SECURITY: &str = "/usr/bin/security";

    pub fn save(service: &str, account: &str, key: &str) -> Result<()> {
        // `security -i` reads its command from standard input, so the key
        // never shows in the process list.
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
                "add-generic-password -U -s \"{service}\" -a \"{account}\" -l \"{service}\" -w \"{key}\""
            )?;
        }
        let output = child.wait_with_output()?;
        // Interactive mode reports a failed command on stderr, not in its
        // status.
        if output.status.success() && String::from_utf8_lossy(&output.stderr).trim().is_empty() {
            Ok(())
        } else {
            Err(eyre!(
                "the Keychain didn't save the key: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    pub fn load(service: &str, account: &str) -> Option<String> {
        let output = Command::new(SECURITY)
            .args(["find-generic-password", "-s", service, "-a", account, "-w"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout).ok()
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let output = Command::new(SECURITY)
            .args(["delete-generic-password", "-s", service, "-a", account])
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
}

#[cfg(target_os = "linux")]
mod system {
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::time::Duration;

    use color_eyre::{Result, eyre::eyre};

    pub const NAME: &str = "the keyring";
    /// How long `secret-tool` may take; a keyring that asks to be unlocked
    /// shows its own prompt meanwhile.
    const WAIT: Duration = Duration::from_secs(60);

    pub fn save(service: &str, account: &str, key: &str) -> Result<()> {
        match secret_tool(
            &[
                "store",
                &format!("--label={service}"),
                "service",
                service,
                "account",
                account,
            ],
            Some(key),
        ) {
            Some(Ok(_)) => {
                // A key saved before a keyring was set up stays out of the
                // file once the keyring holds it.
                let _ = forget_in_file(account);
                Ok(())
            }
            Some(Err(error)) => {
                tracing::warn!(%error, "the keyring didn't save the key; keeping it in a private file");
                save_in_file(account, key)
            }
            None => save_in_file(account, key),
        }
    }

    pub fn load(service: &str, account: &str) -> Option<String> {
        if let Some(Ok(key)) =
            secret_tool(&["lookup", "service", service, "account", account], None)
            && !key.trim().is_empty()
        {
            return Some(key);
        }
        read_file().remove(account)
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        // Nothing saved is fine for `secret-tool clear` too.
        if let Some(Err(error)) =
            secret_tool(&["clear", "service", service, "account", account], None)
        {
            tracing::debug!(%error, "the keyring didn't clear the key");
        }
        forget_in_file(account)
    }

    /// Runs `secret-tool` with `secret` on its standard input; `None` when
    /// it isn't installed.
    fn secret_tool(args: &[&str], secret: Option<&str>) -> Option<Result<String>> {
        let program = crate::assistant::backends::search::find("secret-tool")?;
        Some(run(program, args, secret))
    }

    fn run(program: PathBuf, args: &[&str], secret: Option<&str>) -> Result<String> {
        let mut child = crate::desktop::background_process::command(program)
            .args(args)
            .stdin(if secret.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| eyre!("couldn't start secret-tool: {error}"))?;
        if let (Some(mut stdin), Some(secret)) = (child.stdin.take(), secret) {
            stdin.write_all(secret.as_bytes())?;
        }
        let started = std::time::Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if started.elapsed() > WAIT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(eyre!("the keyring didn't answer"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(eyre!(
                "secret-tool failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    /// The private file keys fall back to: readable by the person alone.
    fn file() -> Result<PathBuf> {
        Ok(crate::app_paths::support_dir()?.join("assistant-keys.json"))
    }

    fn read_file() -> BTreeMap<String, String> {
        file()
            .ok()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn write_file(keys: &BTreeMap<String, String>) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let path = file()?;
        if keys.is_empty() {
            match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => return Ok(()),
            }
        }
        if let Some(dir) = path.parent() {
            crate::app_paths::create_private_dir(dir)?;
        }
        let temporary = path.with_extension("json.tmp");
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        out.write_all(&serde_json::to_vec_pretty(keys)?)?;
        out.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        Ok(())
    }

    fn save_in_file(account: &str, key: &str) -> Result<()> {
        let mut keys = read_file();
        keys.insert(account.to_owned(), key.to_owned());
        write_file(&keys)
    }

    fn forget_in_file(account: &str) -> Result<()> {
        let mut keys = read_file();
        if keys.remove(account).is_some() {
            write_file(&keys)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod system {
    use color_eyre::{Result, eyre::eyre};
    use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    pub const NAME: &str = "Credential Manager";

    /// `Agent-Desk Assistant/anthropic`, as Credential Manager lists it.
    fn target(service: &str, account: &str) -> Vec<u16> {
        format!("{service}/{account}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn save(service: &str, account: &str, key: &str) -> Result<()> {
        let mut target = target(service, account);
        let mut user = wide(account);
        let mut blob = key.as_bytes().to_vec();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: u32::try_from(blob.len())
                .map_err(|_| eyre!("that key is too long"))?,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: user.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: every pointer in `credential` points into a buffer that
        // outlives the call; Credential Manager copies them.
        let saved = unsafe { CredWriteW(&credential, 0) };
        blob.fill(0);
        if saved == 0 {
            // SAFETY: reads this thread's last error.
            let error = unsafe { GetLastError() };
            return Err(eyre!(
                "Credential Manager didn't save the key (error {error})"
            ));
        }
        Ok(())
    }

    pub fn load(service: &str, account: &str) -> Option<String> {
        let target = target(service, account);
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated; on success `credential` points
        // to memory Credential Manager owns until `CredFree`.
        let found = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
        if found == 0 || credential.is_null() {
            return None;
        }
        // SAFETY: `credential` is valid until freed below; its blob holds
        // `CredentialBlobSize` bytes.
        let key = unsafe {
            let credential = &*credential;
            let bytes = std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            );
            String::from_utf8(bytes.to_vec()).ok()
        };
        // SAFETY: frees what CredReadW allocated.
        unsafe { CredFree(credential.cast()) };
        key
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let target = target(service, account);
        // SAFETY: `target` is NUL-terminated.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
            // SAFETY: reads this thread's last error.
            let error = unsafe { GetLastError() };
            if error != ERROR_NOT_FOUND {
                return Err(eyre!(
                    "Credential Manager didn't delete the key (error {error})"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod system {
    use color_eyre::{Result, eyre::eyre};

    pub const NAME: &str = "a key store";

    pub fn save(_service: &str, _account: &str, _key: &str) -> Result<()> {
        Err(eyre!("this computer has nowhere to keep keys"))
    }

    pub fn load(_service: &str, _account: &str) -> Option<String> {
        None
    }

    pub fn delete(_service: &str, _account: &str) -> Result<()> {
        Ok(())
    }
}
