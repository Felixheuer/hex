//! The person's accounts in Agent-Desk's hidden browsers: signed in once, then
//! every task's browser starts with them.
//!
//! Two ways in, both into the one sign-in state every browser restores
//! (`hex-sign-in`, [`super::saved_sites`]):
//! - **From Chrome** ([`import_from_chrome`]): the cookies of one of the
//!   person's Chrome profiles, read from a copy of its `Cookies` database
//!   and decrypted with Chrome's own key from the login Keychain ("Chrome
//!   Safe Storage"; macOS asks once whether Agent-Desk may read it). They are
//!   merged into the sign-in state, newest wins. Re-imported when older than
//!   a day ([`refresh_if_stale`]), off the main thread.
//!
//!   Google's and YouTube's sign-in cookies stay behind ([`kept_in_chrome`]):
//!   Google binds a session to the browser that holds it, rotates its
//!   cookies as it is used, and ends a session it sees used from a second
//!   browser at once, everywhere. Twenty hidden Chromes using a copy of the
//!   person's Google session signed them out of Google and YouTube in their
//!   own Chrome. Their consent choice (`SOCS`, `CONSENT`) still comes over,
//!   and the person may sign in to Google by hand in Agent-Desk's window:
//!   that is a session of its own, which leaves Chrome's alone. A state that
//!   held copies from before is cleaned once ([`keep_google_out`]).
//! - **By hand** in Agent-Desk's visible sign-in window ([`super::sign_in`]).
//!
//! The state is a session-token file only this user may read; nothing is
//! logged but counts and site names. Chrome itself is never touched: its
//! database is copied (Chrome may keep running) and the copy deleted.
//!
//! OWNER: browser.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use color_eyre::{Result, eyre::eyre};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// How old an import may get before it is taken again.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// Seconds between 1601-01-01 (Chrome's epoch) and 1970-01-01.
const WINDOWS_TO_UNIX: i64 = 11_644_473_600;

/// A Chrome profile: its folder (`Default`, `Profile 1`) and the name the
/// person gave it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile {
    pub directory: String,
    pub name: String,
}

/// Where the last import came from and when.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Imported {
    pub profile: String,
    pub at_ms: u64,
    /// Sites with cookies that came over.
    #[serde(default)]
    pub sites: Vec<String>,
    /// Google's sign-in cookies were kept out of this import, and copies an
    /// older one left in the state were removed ([`keep_google_out`]).
    #[serde(default)]
    pub google_kept_out: bool,
}

/// Chrome's profiles, from its `Local State`; empty when Chrome isn't here.
pub fn chrome_profiles() -> Vec<Profile> {
    chrome_dir()
        .and_then(|dir| std::fs::read(dir.join("Local State")).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|state| profiles_in(&state))
        .unwrap_or_default()
}

fn profiles_in(local_state: &Value) -> Vec<Profile> {
    let mut profiles: Vec<Profile> = local_state["profile"]["info_cache"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(directory, info)| Profile {
            directory: directory.clone(),
            name: info["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or(directory)
                .to_owned(),
        })
        .collect();
    // Default first, then as Chrome numbers them.
    profiles.sort_by_key(|profile| {
        (
            profile.directory != "Default",
            profile
                .directory
                .rsplit(' ')
                .next()
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(0),
        )
    });
    profiles
}

fn chrome_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(crate::app_paths::home_dir()?.join("Library/Application Support/Google/Chrome"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Takes the cookies of Chrome's `profile` into the sign-in state; returns
/// the sites they are for. Blocks (Keychain, a database copy); off the UI
/// thread. macOS only: Chrome elsewhere keeps its key differently.
pub fn import_from_chrome(profile: &str) -> Result<Vec<String>> {
    if profile.contains('/') || profile.contains("..") || profile.is_empty() {
        return Err(eyre!("not a Chrome profile: {profile}"));
    }
    let database = chrome_dir()
        .ok_or_else(|| eyre!("Chrome isn't installed"))?
        .join(profile)
        .join("Cookies");
    if !database.exists() {
        return Err(eyre!("Chrome's profile “{profile}” has no cookies yet"));
    }
    let key = cookie_key(&chrome_password()?);
    let rows = read_cookies(&database)?;
    let now = unix_now();
    let cookies: Vec<Value> = rows
        .iter()
        .filter(|row| !kept_in_chrome(&row.host_key, &row.name))
        .filter_map(|row| cookie(row, &key, now))
        .collect();
    if cookies.is_empty() {
        return Err(eyre!("no signed-in cookies came over from Chrome"));
    }
    // Only an import from before copied Google's sign-in; with no record,
    // a Google cookie in the state is a sign-in made by hand, and stays.
    let copied_google = last_import().is_some_and(|last| !last.google_kept_out);
    let sites = merge_into_sign_in(&cookies, copied_google)?;
    let record = Imported {
        profile: profile.to_owned(),
        at_ms: (now * 1000.0) as u64,
        sites: sites.clone(),
        google_kept_out: true,
    };
    if let Some(path) = record_path() {
        let _ = std::fs::write(&path, serde_json::to_vec(&record)?);
    }
    tracing::info!(
        cookies = cookies.len(),
        sites = sites.len(),
        "took the person's Chrome logins into Agent-Desk's browsers"
    );
    Ok(sites)
}

/// The last import, if any.
pub fn last_import() -> Option<Imported> {
    let bytes = std::fs::read(record_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Forgets the import record (with [`super::forget_sign_in`]).
pub fn forget_import() {
    if let Some(path) = record_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether an import taken at `at_ms` is old enough to take again at
/// `now_ms`.
pub fn stale(at_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(at_ms) >= STALE_AFTER.as_millis() as u64
}

/// Takes Chrome's logins again when the last import from `profile` (the
/// setting) is stale. Called on Agent-Desk's launch, on its own thread.
/// First removes the copies of Google's sign-in an older import left
/// ([`keep_google_out`]), even when no profile is chosen any more.
pub fn refresh_if_stale(profile: Option<&str>) {
    keep_google_out();
    let Some(profile) = profile.filter(|profile| !profile.is_empty()) else {
        return;
    };
    let now_ms = (unix_now() * 1000.0) as u64;
    let due = last_import().is_none_or(|last| last.profile != profile || stale(last.at_ms, now_ms));
    if !due {
        return;
    }
    if let Err(error) = import_from_chrome(profile) {
        tracing::warn!(%error, "couldn't refresh the Chrome logins");
    }
}

/// Whether the cookie `name` on `host` stays in Chrome: a Google or YouTube
/// cookie, except the consent choice. See the module's note.
pub fn kept_in_chrome(host: &str, name: &str) -> bool {
    google_host(host) && !matches!(name, "SOCS" | "CONSENT")
}

/// `google.com`, `accounts.google.com`, `.google.de`, `google.co.uk`,
/// `youtube.com`, `m.youtube.com`, `youtube-nocookie.com`: Google's own
/// sign-in domains, wherever Chrome kept a cookie for them.
fn google_host(host: &str) -> bool {
    let host = host.trim_start_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    // `google` followed by a public suffix of one or two short labels
    // (`com`, `de`, `co.uk`, `com.br`).
    let google = labels.iter().enumerate().any(|(at, label)| {
        let suffix = &labels[at + 1..];
        *label == "google"
            && (1..=2).contains(&suffix.len())
            && suffix.iter().all(|part| (2..=3).contains(&part.len()))
    });
    google
        || matches!(
            labels.as_slice(),
            [.., "youtube" | "youtube-nocookie", "com"]
        )
}

/// One-time cleanup, remove after 2027-02: imports before 2026-11-13 copied
/// the person's Google session into every task's browser, which signed
/// them out of Google and YouTube in Chrome. When the last import did, its
/// Google cookies leave the sign-in state now (sign-ins made by hand in
/// Agent-Desk's window afterwards stay). Every Agent-Desk browser still
/// running from before closes first: they hold the copies in memory, and
/// the sign-in window saves its state as it closes. Marked done only once
/// the state is clean (or there is none), so a failed try comes again on
/// the next launch.
pub fn keep_google_out() {
    let Some(mut last) = last_import() else {
        return;
    };
    if last.google_kept_out {
        return;
    }
    if let Some(program) = super::executable()
        && let Err(error) = super::call(&program, super::SIGN_IN, &["close", "--all"])
    {
        tracing::debug!(%error, "no Agent-Desk browser to close before the cleanup");
    }
    let Some(path) = super::sign_in_state_file() else {
        return;
    };
    let _state = state_lock();
    match std::fs::read(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(%error, "couldn't read the sign-in state to remove the copied Google sign-in");
            return;
        }
        Ok(bytes) => {
            let Ok(mut state) = serde_json::from_slice::<Value>(&bytes) else {
                tracing::warn!(
                    "the sign-in state isn't readable now; the Google cleanup tries again next time"
                );
                return;
            };
            let removed = remove_google(&mut state);
            if removed > 0 {
                let written = serde_json::to_vec(&state)
                    .map_err(|error| eyre!("{error}"))
                    .and_then(|bytes| write_private(&path, &bytes));
                if let Err(error) = written {
                    tracing::warn!(%error, "couldn't remove the copied Google sign-in");
                    return;
                }
                tracing::info!(
                    removed,
                    "removed copies of the person's Google sign-in from Agent-Desk's browsers"
                );
            }
            last.sites = super::sites_in_state(&state);
        }
    }
    last.google_kept_out = true;
    if let Some(record) = record_path()
        && let Ok(bytes) = serde_json::to_vec(&last)
    {
        let _ = std::fs::write(record, bytes);
    }
}

/// Held while Agent-Desk itself reads, changes and writes the sign-in state
/// (an import, the cleanup), so neither overwrites the other's change.
fn state_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

/// Removes the cookies [`kept_in_chrome`] keeps out from `state`; how many.
fn remove_google(state: &mut Value) -> usize {
    let Some(cookies) = state["cookies"].as_array_mut() else {
        return 0;
    };
    let before = cookies.len();
    cookies.retain(|cookie| {
        !kept_in_chrome(
            cookie["domain"].as_str().unwrap_or_default(),
            cookie["name"].as_str().unwrap_or_default(),
        )
    });
    before - cookies.len()
}

/// Writes `bytes` to `path` whole or not at all (a copy beside it, private
/// before it holds anything, then renamed over it), so a browser starting
/// meanwhile never reads half a state.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let name = path
        .file_name()
        .ok_or_else(|| eyre!("no file name"))?
        .to_string_lossy();
    let partial = path.with_file_name(format!(".{name}.{}.partial", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&partial)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&partial, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(error.into());
    }
    Ok(())
}

fn record_path() -> Option<PathBuf> {
    Some(
        crate::assistant::store::assistant_dir()
            .ok()?
            .join("browser-import.json"),
    )
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// Chrome's cookie password from the login Keychain; macOS shows its own
/// "allow" prompt the first time.
fn chrome_password() -> Result<String> {
    let output = std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            "Chrome Safe Storage",
            "-a",
            "Chrome",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()?;
    let password = String::from_utf8(output.stdout)?.trim_end().to_owned();
    if !output.status.success() || password.is_empty() {
        return Err(eyre!(
            "macOS didn't let Agent-Desk read Chrome's key; allow it when asked"
        ));
    }
    Ok(password)
}

/// Chrome's AES key on macOS: PBKDF2-HMAC-SHA1 of the Keychain password.
fn cookie_key(password: &str) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), b"saltysalt", 1003, &mut key);
    key
}

/// One row of Chrome's `cookies` table, as `sqlite3 -json` prints it.
#[derive(Debug, Deserialize)]
struct Row {
    host_key: String,
    name: String,
    path: String,
    expires_utc: i64,
    is_secure: i64,
    is_httponly: i64,
    samesite: i64,
    /// `encrypted_value` in hex.
    ev: String,
    /// The `meta` table's version: from 24 on, a value starts with the
    /// SHA-256 of its host.
    version: i64,
}

/// The rows of a copy of `database`, read with the system's `sqlite3`.
fn read_cookies(database: &Path) -> Result<Vec<Row>> {
    let scratch = tempfile_dir()?;
    let copy = scratch.join("Cookies");
    std::fs::copy(database, &copy)?;
    for suffix in ["-wal", "-shm"] {
        let side = database.with_file_name(format!("Cookies{suffix}"));
        if side.exists() {
            let _ = std::fs::copy(&side, scratch.join(format!("Cookies{suffix}")));
        }
    }
    let output = std::process::Command::new("/usr/bin/sqlite3")
        .arg("-json")
        .arg(&copy)
        .arg(
            "SELECT host_key, name, path, expires_utc, is_secure, is_httponly, samesite, \
             hex(encrypted_value) AS ev, \
             (SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'version') AS version \
             FROM cookies",
        )
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let _ = std::fs::remove_dir_all(&scratch);
    let output = output?;
    if !output.status.success() {
        return Err(eyre!("couldn't read Chrome's cookies"));
    }
    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// A private folder for the database copy.
fn tempfile_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "hex-chrome-import-{}-{}",
        std::process::id(),
        (unix_now() * 1000.0) as u64
    ));
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

/// The CDP cookie for `row`, or `None` when it can't be read or expired.
fn cookie(row: &Row, key: &[u8; 16], now: f64) -> Option<Value> {
    let encrypted = hex_bytes(&row.ev)?;
    let value = decrypt(&encrypted, key, row.version >= 24)?;
    let expires = if row.expires_utc == 0 {
        -1.0
    } else {
        row.expires_utc as f64 / 1_000_000.0 - WINDOWS_TO_UNIX as f64
    };
    if expires > 0.0 && expires < now {
        return None;
    }
    let same_site = match row.samesite {
        0 => Some("None"),
        1 => Some("Lax"),
        2 => Some("Strict"),
        _ => None,
    };
    let mut cookie = json!({
        "name": row.name,
        "value": value,
        "domain": row.host_key,
        "path": row.path,
        "expires": expires,
        "size": row.name.len() + value.len(),
        "httpOnly": row.is_httponly != 0,
        "secure": row.is_secure != 0,
        "session": row.expires_utc == 0,
    });
    if let Some(same_site) = same_site {
        cookie["sameSite"] = same_site.into();
    }
    Some(cookie)
}

fn hex_bytes(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// A `v10` value: AES-128-CBC with an IV of sixteen spaces; from database
/// version 24 the plaintext starts with a 32-byte hash of the host.
fn decrypt(encrypted: &[u8], key: &[u8; 16], hashed_host: bool) -> Option<String> {
    use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    let body = encrypted.strip_prefix(b"v10")?;
    let plain = cbc::Decryptor::<aes::Aes128>::new(key.into(), &[b' '; 16].into())
        .decrypt_padded_vec_mut::<Pkcs7>(body)
        .ok()?;
    let plain = if hashed_host && plain.len() >= 32 {
        &plain[32..]
    } else {
        &plain[..]
    };
    String::from_utf8(plain.to_vec()).ok()
}

/// Merges `cookies` into the sign-in state (one file every browser
/// restores), replacing those with the same name, domain and path; returns
/// the sites the state then has cookies for. `without_google`: first
/// removes the Google sign-in an older import copied ([`keep_google_out`]).
fn merge_into_sign_in(cookies: &[Value], without_google: bool) -> Result<Vec<String>> {
    let path = super::sign_in_state_file().ok_or_else(|| eyre!("no home folder"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _state = state_lock();
    let mut state = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_else(|| json!({"cookies": [], "origins": []}));
    if without_google {
        remove_google(&mut state);
    }
    merge(&mut state, cookies);
    write_private(&path, &serde_json::to_vec(&state)?)?;
    super::private_state();
    Ok(super::sites_in_state(&state))
}

fn merge(state: &mut Value, cookies: &[Value]) {
    let identity = |cookie: &Value| {
        (
            cookie["name"].as_str().unwrap_or_default().to_owned(),
            cookie["domain"].as_str().unwrap_or_default().to_owned(),
            cookie["path"].as_str().unwrap_or_default().to_owned(),
        )
    };
    let incoming: std::collections::HashSet<_> = cookies.iter().map(identity).collect();
    let mut merged: Vec<Value> = state["cookies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|cookie| !incoming.contains(&identity(cookie)))
        .cloned()
        .collect();
    merged.extend(cookies.iter().cloned());
    state["cookies"] = Value::Array(merged);
    if !state["origins"].is_array() {
        state["origins"] = json!([]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn googles_sign_in_stays_in_chrome() {
        for host in [
            ".google.com",
            "accounts.google.com",
            "mail.google.com",
            ".google.de",
            ".google.co.uk",
            "www.google.com.br",
            ".youtube.com",
            "m.youtube.com",
            ".youtube-nocookie.com",
        ] {
            for name in ["SID", "__Secure-1PSIDTS", "SAPISID", "LOGIN_INFO", "NID"] {
                assert!(kept_in_chrome(host, name), "{host} {name}");
            }
            // The consent choice comes over, so no consent wall.
            assert!(!kept_in_chrome(host, "SOCS"), "{host}");
            assert!(!kept_in_chrome(host, "CONSENT"), "{host}");
        }
        for host in [
            "github.com",
            ".notion.so",
            "googlesyndication.com",
            "google-analytics.com",
            "mygoogle.example.com",
            "youtube.example.org",
            "docs.example.google.internal",
        ] {
            assert!(!kept_in_chrome(host, "SID"), "{host}");
        }
    }

    #[test]
    fn a_copied_google_sign_in_is_removed_from_the_state() {
        let cookie = |domain: &str, name: &str| json!({"name": name, "domain": domain, "path": "/", "value": "v"});
        let mut state = json!({"cookies": [
            cookie(".google.com", "SID"),
            cookie(".google.com", "SOCS"),
            cookie(".youtube.com", "LOGIN_INFO"),
            cookie("github.com", "user_session"),
        ], "origins": []});
        assert_eq!(remove_google(&mut state), 2);
        let left: Vec<&str> = state["cookies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|cookie| cookie["name"].as_str().unwrap())
            .collect();
        assert_eq!(left, ["SOCS", "user_session"]);
        assert_eq!(remove_google(&mut state), 0);
    }

    #[test]
    fn the_state_is_written_whole_and_private() {
        let dir = std::env::temp_dir().join(format!("hex-state-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hex-sign-in-hex-sign-in.json");
        std::fs::write(&path, b"old").unwrap();
        write_private(&path, b"{\"cookies\":[]}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"cookies\":[]}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // No partial copy is left beside it.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_import_record_from_before_says_google_came_over() {
        let old: Imported =
            serde_json::from_str(r#"{"profile":"Default","at_ms":1,"sites":["google.com"]}"#)
                .unwrap();
        assert!(!old.google_kept_out);
    }

    #[test]
    fn chrome_profiles_come_from_local_state_default_first() {
        let state = json!({"profile": {"info_cache": {
            "Profile 2": {"name": "Arbeit"},
            "Default": {"name": "felix"},
            "Profile 1": {"name": ""},
        }}});
        assert_eq!(
            profiles_in(&state),
            [
                Profile {
                    directory: "Default".into(),
                    name: "felix".into()
                },
                Profile {
                    directory: "Profile 1".into(),
                    name: "Profile 1".into()
                },
                Profile {
                    directory: "Profile 2".into(),
                    name: "Arbeit".into()
                },
            ]
        );
        assert!(profiles_in(&json!({})).is_empty());
    }

    fn encrypt(plain: &[u8], key: &[u8; 16]) -> Vec<u8> {
        use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
        let mut out = b"v10".to_vec();
        out.extend(
            cbc::Encryptor::<aes::Aes128>::new(key.into(), &[b' '; 16].into())
                .encrypt_padded_vec_mut::<Pkcs7>(plain),
        );
        out
    }

    #[test]
    fn a_v10_value_decrypts_with_and_without_the_host_hash() {
        let key = cookie_key("not-a-real-password");
        assert_eq!(
            decrypt(&encrypt(b"abc", &key), &key, false).as_deref(),
            Some("abc")
        );
        let mut hashed = vec![7u8; 32];
        hashed.extend(b"token");
        assert_eq!(
            decrypt(&encrypt(&hashed, &key), &key, true).as_deref(),
            Some("token")
        );
        assert_eq!(decrypt(b"v11whatever", &key, false), None);
        assert_eq!(
            decrypt(&encrypt(b"abc", &cookie_key("other")), &key, false),
            None
        );
    }

    #[test]
    fn a_row_becomes_a_cdp_cookie_and_expired_ones_drop() {
        let key = cookie_key("k");
        let hex: String = encrypt(b"v", &key)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect();
        let row = |expires_utc| Row {
            host_key: ".github.com".into(),
            name: "user_session".into(),
            path: "/".into(),
            expires_utc,
            is_secure: 1,
            is_httponly: 1,
            samesite: 1,
            ev: hex.clone(),
            version: 0,
        };
        // 2030-01-01 in Chrome's microseconds since 1601.
        let future = (1_893_456_000 + WINDOWS_TO_UNIX) * 1_000_000;
        let made = cookie(&row(future), &key, 1_800_000_000.0).unwrap();
        assert_eq!(made["domain"], ".github.com");
        assert_eq!(made["sameSite"], "Lax");
        assert_eq!(made["expires"], 1_893_456_000.0);
        assert_eq!(made["httpOnly"], true);
        let session = cookie(&row(0), &key, 1_800_000_000.0).unwrap();
        assert_eq!(session["expires"], -1.0);
        assert_eq!(session["session"], true);
        let past = (1_600_000_000 + WINDOWS_TO_UNIX) * 1_000_000;
        assert!(cookie(&row(past), &key, 1_800_000_000.0).is_none());
    }

    #[test]
    fn imported_cookies_replace_the_same_cookie_and_keep_the_rest() {
        let mut state = json!({"cookies": [
            {"name": "a", "domain": ".x.com", "path": "/", "value": "old"},
            {"name": "keep", "domain": ".y.com", "path": "/", "value": "1"},
        ], "origins": []});
        merge(
            &mut state,
            &[json!({"name": "a", "domain": ".x.com", "path": "/", "value": "new"})],
        );
        let cookies = state["cookies"].as_array().unwrap();
        assert_eq!(cookies.len(), 2);
        assert!(cookies.iter().any(|c| c["value"] == "new"));
        assert!(cookies.iter().any(|c| c["name"] == "keep"));
    }

    #[test]
    fn an_import_goes_stale_after_a_day() {
        let day = STALE_AFTER.as_millis() as u64;
        assert!(!stale(1_000, 1_000 + day - 1));
        assert!(stale(1_000, 1_000 + day));
    }
}
