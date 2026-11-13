//! macOS installs: an `Agent-Desk.app` this user can replace updates itself.
//! The update ZIP must carry Pilot's release signature (`super::signature`),
//! is unpacked beside the installed bundle (same volume), its code signature
//! must be valid for `fheuer.agentdesk` and, when the build names its Apple
//! team (`AGENT_DESK_UPDATE_TEAM_ID`), come from that team, and on "Restart to
//! update" (or quit) the two bundles swap atomically (`renamex_np` with
//! `RENAME_SWAP`). The old bundle stays beside it until the new version
//! launches and removes it. A read-only, translocated, or root-owned location
//! links to the DMG instead of escalating. Administrators turn updates off
//! with `DisableUpdates` in the `fheuer.agentdesk` defaults domain.
//!
//! One-time migration, remove after 2026-12: an installed Hex
//! (`/Applications/Hex.app`, `anomaly.hex`) only accepts an update with its
//! own identity, so its channel's `macos-arm64` target is a bridge: a
//! `Hex.app` with the old identity that carries the real `Agent-Desk.app` in
//! `Contents/Library`. Launched, the bridge installs that bundle beside
//! itself and opens it ([`hand_off_from_hex`]), and Agent-Desk removes the
//! bridge and what the old updater left ([`finish_on_launch`]).

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use objc2_foundation::{NSString, NSUserDefaults};

use super::Method;

/// Agent-Desk's own update target; `macos-arm64` is the bridge older Hex
/// installs take (`scripts/pilot-release-macos.sh`).
pub const TARGET: &str = "macos-arm64-agent-desk";
pub const DOWNLOAD_TARGET: &str = "macos-arm64-dmg";
const BUNDLE_ID: &str = crate::app_paths::Variant::Live.bundle_id();
/// The bundle's folder name, in the update ZIP and in /Applications.
const BUNDLE_NAME: &str = "Agent-Desk.app";
/// The Apple team (its 10-character ID) Pilot's Mac releases are signed by,
/// stamped at build time by `scripts/pilot-release-macos.sh`. The same
/// team's Apple Development or Developer ID certificate keeps macOS
/// permissions across updates.
const TEAM_ID: Option<&str> = option_env!("AGENT_DESK_UPDATE_TEAM_ID");

/// The code requirement an update must satisfy: `fheuer.agentdesk`, signed by
/// `team` through Apple's CA when the build names one. Without a team (an
/// ad-hoc signed build) only the identifier and an intact signature are
/// checked; Pilot's release signature still vouches for the bytes.
fn requirement_for(team: Option<&str>) -> String {
    match team.filter(|team| {
        team.len() == 10
            && team
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    }) {
        Some(team) => format!(
            "anchor apple generic and identifier \"{BUNDLE_ID}\" and certificate leaf[subject.OU] = \"{team}\""
        ),
        None => format!("identifier \"{BUNDLE_ID}\""),
    }
}

/// Unpacked updates (and, after a swap, the replaced bundle) live in
/// `.<name>-update-<version>` beside the installed bundle.
const STAGING_PREFIX: &str = ".Agent-Desk-update-";

/// An unpacked, verified bundle and the installed bundle it replaces.
#[derive(Clone, Debug)]
pub struct Staged {
    update: PathBuf,
    installed: PathBuf,
}

/// The running `Agent-Desk.app`, when this executable is inside one.
fn installed_bundle() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bundle = executable.parent()?.parent()?.parent()?;
    (bundle
        .extension()
        .is_some_and(|extension| extension == "app")
        && bundle.join("Contents/Info.plist").is_file())
    .then(|| bundle.to_path_buf())
}

/// Only Windows explains beside Download update why it can't update itself.
pub fn blocked() -> Option<&'static str> {
    None
}

pub fn method() -> Method {
    let Some(bundle) = installed_bundle() else {
        // A bare binary from a checkout.
        return Method::Source;
    };
    if replaceable(&bundle) {
        // The app carries no receipt; its build channel or `updates.channel`
        // picks another channel (`super::channel`).
        Method::Direct {
            channel: "stable".into(),
        }
    } else {
        Method::DownloadOnly
    }
}

/// The `macos-arm64` target is a zip of the bundle, which the running app
/// cannot reproduce, so a Mac build is identified by its version text.
pub fn running_sha256() -> Option<String> {
    None
}

/// Agent-Desk replaces the bundle by renaming it inside its parent directory, so
/// both must be writable by this user, on a writable volume, and not a
/// Gatekeeper App Translocation copy.
fn replaceable(bundle: &Path) -> bool {
    let writable = |path: &Path| {
        CString::new(path.as_os_str().as_bytes())
            // access has no side effects; the string is NUL-terminated.
            .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0)
    };
    !bundle
        .components()
        .any(|component| component.as_os_str() == "AppTranslocation")
        && bundle.parent().is_some_and(writable)
        && writable(bundle)
}

pub fn updates_disabled_by_policy() -> bool {
    NSUserDefaults::standardUserDefaults().boolForKey(&NSString::from_str("DisableUpdates"))
}

/// Removes what an earlier update left beside the bundle: the replaced
/// bundle once this (new) version runs, or an unpacked update never applied.
pub fn finish_on_launch() {
    let Some(parent) = installed_bundle().and_then(|bundle| bundle.parent().map(Path::to_path_buf))
    else {
        return;
    };
    remove_staging(&parent, STAGING_PREFIX);
    if crate::app_paths::variant() == crate::app_paths::Variant::Live {
        remove_hex(&parent);
    }
}

fn remove_staging(parent: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(prefix))
            && let Err(error) = fs::remove_dir_all(entry.path())
        {
            tracing::warn!(%error, path = %entry.path().display(), "could not remove an earlier update");
        }
    }
}

/// One-time migration, remove after 2026-12: removes the Hex an update
/// replaced from `parent` (`/Applications`): the bridge `Hex.app` (only if it
/// still has Hex's identity) and what Hex's own updater left beside it
/// (`.Hex-update-<version>`), and tells LaunchServices it is gone.
fn remove_hex(parent: &Path) {
    use crate::app_paths::{Variant, legacy};
    remove_staging(parent, legacy::UPDATE_STAGING_PREFIX);
    let old = legacy::identity(Variant::Live);
    let bundle = parent.join(format!("{}.app", old.name));
    if plist_value(&bundle, "CFBundleIdentifier").ok().as_deref() != Some(old.bundle_id) {
        return;
    }
    match fs::remove_dir_all(&bundle) {
        Ok(()) => {
            tracing::info!(path = %bundle.display(), "removed the Hex that Agent-Desk replaced");
            let _ = Command::new(LSREGISTER)
                .arg("-u")
                .arg(&bundle)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        Err(error) => tracing::warn!(%error, path = %bundle.display(), "could not remove Hex"),
    }
}

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// One-time migration, remove after 2026-12: when this process runs as the
/// bridge (a bundle with Hex's identity carrying `Contents/Library/
/// Agent-Desk.app`), installs that bundle beside it, opens it, and returns
/// true: the caller exits at once, before any data folder, lock, or
/// permission is touched. Anything else returns false.
pub fn hand_off_from_hex() -> bool {
    let Some(bridge) = installed_bundle() else {
        return false;
    };
    let old = crate::app_paths::legacy::identity(crate::app_paths::Variant::Live);
    if plist_value(&bridge, "CFBundleIdentifier").ok().as_deref() != Some(old.bundle_id) {
        return false;
    }
    let carried = bridge.join("Contents/Library").join(BUNDLE_NAME);
    if !carried.is_dir() {
        return false;
    }
    let result = install_carried(&carried, &requirement_for(TEAM_ID)).and_then(|installed| {
        let status = Command::new("/usr/bin/open")
            .arg(&installed)
            .stdin(Stdio::null())
            .status()
            .wrap_err("could not open Agent-Desk")?;
        if !status.success() {
            bail!("open {} failed ({status})", installed.display());
        }
        Ok(())
    });
    if let Err(error) = result {
        // Nothing went: the bridge stays, and the next launch tries again.
        eprintln!("Agent-Desk could not replace Hex: {error:#}");
    }
    true
}

/// Copies the bundle the bridge carries to `Agent-Desk.app` beside the
/// bridge, checked like an update (identity, its own version, `requirement`)
/// before it replaces anything. An `Agent-Desk.app` already there at the same
/// version stays as it is; another version is swapped out and removed.
/// Returns the installed bundle.
fn install_carried(carried: &Path, requirement: &str) -> Result<PathBuf> {
    let bridge = carried
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| eyre!("the bridge has no bundle"))?;
    let parent = bridge
        .parent()
        .ok_or_else(|| eyre!("the bridge has no parent folder"))?;
    let version = plist_value(carried, "CFBundleShortVersionString")?;
    let installed = parent.join(BUNDLE_NAME);
    if plist_value(&installed, "CFBundleShortVersionString")
        .ok()
        .as_deref()
        == Some(&version)
        && plist_value(&installed, "CFBundleIdentifier")
            .ok()
            .as_deref()
            == Some(BUNDLE_ID)
    {
        return Ok(installed);
    }
    let staging = parent.join(format!("{STAGING_PREFIX}{version}"));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;
    let update = staging.join(BUNDLE_NAME);
    let copied = Command::new("/usr/bin/ditto")
        .arg(carried)
        .arg(&update)
        .stdin(Stdio::null())
        .status()
        .wrap_err("could not run ditto")?;
    let checked = if copied.success() {
        verify_bundle(&update, &version, requirement)
    } else {
        Err(eyre!("could not copy Agent-Desk ({copied})"))
    };
    if let Err(error) = checked {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if installed.exists() {
        swap(&Staged {
            update,
            installed: installed.clone(),
        })?;
    } else {
        fs::rename(&update, &installed)?;
    }
    let _ = fs::remove_dir_all(&staging);
    Ok(installed)
}

pub fn stage(archive: &Path, version: &str) -> Result<Staged> {
    let installed =
        installed_bundle().ok_or_else(|| eyre!("Agent-Desk is not running from {BUNDLE_NAME}"))?;
    let parent = installed
        .parent()
        .ok_or_else(|| eyre!("{BUNDLE_NAME} has no parent folder"))?;
    let staging = parent.join(format!("{STAGING_PREFIX}{version}"));
    unpack(archive, &staging)?;
    let update = staging.join(BUNDLE_NAME);
    let result = verify_bundle(&update, version, &requirement_for(TEAM_ID));
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    Ok(Staged { update, installed })
}

fn unpack(archive: &Path, staging: &Path) -> Result<()> {
    if staging.exists() {
        fs::remove_dir_all(staging)?;
    }
    fs::create_dir(staging)?;
    let status = Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(archive)
        .arg(staging)
        .stdin(Stdio::null())
        .status()
        .wrap_err("could not run ditto")?;
    if !status.success() {
        let _ = fs::remove_dir_all(staging);
        bail!("could not unpack the update ({status})");
    }
    Ok(())
}

/// Checks the unpacked bundle's identity, version, and code signature
/// against `requirement` before anything runs from it.
/// `key` in `bundle`'s Info.plist.
fn plist_value(bundle: &Path, key: &str) -> Result<String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(format!("Print :{key}"))
        .arg(bundle.join("Contents/Info.plist"))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        bail!("{}'s Info.plist has no {key}", bundle.display());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn verify_bundle(bundle: &Path, version: &str, requirement: &str) -> Result<()> {
    let value = |key: &str| plist_value(bundle, key);
    if value("CFBundleIdentifier")? != BUNDLE_ID {
        bail!("the update is not {BUNDLE_ID}");
    }
    if value("CFBundleShortVersionString")? != version {
        bail!("the update is not version {version}");
    }
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(format!("-R={}", requirement.trim()))
        .arg(bundle)
        .stdin(Stdio::null())
        .output()
        .wrap_err("could not run codesign")?;
    if !output.status.success() {
        bail!(
            "the update's code signature does not satisfy Pilot's signing requirement: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Swaps the two bundles in one step: afterwards the installed path holds
/// the update and the staging folder the previous version.
fn swap(staged: &Staged) -> Result<()> {
    let from = CString::new(staged.update.as_os_str().as_bytes())?;
    let to = CString::new(staged.installed.as_os_str().as_bytes())?;
    // Both paths are NUL-terminated and outlive the call.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } != 0 {
        return Err(std::io::Error::last_os_error())
            .wrap_err("could not swap in the updated Agent-Desk.app");
    }
    Ok(())
}

/// Swaps in the update and opens it once this process has exited.
pub fn restart_into(staged: &Staged) -> Result<()> {
    swap(staged)?;
    Command::new("/bin/sh")
        .args([
            "-c",
            "while kill -0 \"$AGENT_DESK_PID\" 2>/dev/null; do sleep 0.2; done; exec /usr/bin/open \"$AGENT_DESK_BUNDLE\"",
        ])
        .env("AGENT_DESK_PID", std::process::id().to_string())
        .env("AGENT_DESK_BUNDLE", &staged.installed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .wrap_err("could not schedule the updated Agent-Desk")?;
    Ok(())
}

/// Quitting with an update ready installs it for the next launch.
pub fn install_on_quit(staged: &Staged) {
    if let Err(error) = swap(staged) {
        tracing::warn!(error = %format!("{error:#}"), "could not install the update at quit");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agent-desk-macos-update-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    }

    fn plain(identifier: &str) -> String {
        format!("identifier \"{identifier}\"")
    }

    #[test]
    fn the_team_requirement_compiles() {
        for team in [None, Some("ABCDE12345"), TEAM_ID] {
            let status = Command::new("/usr/bin/csreq")
                .arg(format!("-r={}", requirement_for(team)))
                .arg("-t")
                .stdout(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "{team:?}");
        }
        assert_eq!(requirement_for(Some("bad")), requirement_for(None));
    }

    /// A minimal ad-hoc signed `<name>` bundle with `identifier` and `version`.
    fn named_bundle(root: &Path, name: &str, identifier: &str, version: &str) -> PathBuf {
        let bundle = root.join(name);
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::copy("/usr/bin/true", bundle.join("Contents/MacOS/agent-desk")).unwrap();
        fs::write(
            bundle.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleIdentifier</key><string>{identifier}</string>\
                 <key>CFBundleExecutable</key><string>agent-desk</string>\
                 <key>CFBundleShortVersionString</key><string>{version}</string>\
                 </dict></plist>"
            ),
        )
        .unwrap();
        let status = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--identifier", identifier])
            .arg(&bundle)
            .status()
            .unwrap();
        assert!(status.success());
        bundle
    }

    fn bundle(root: &Path, identifier: &str, version: &str) -> PathBuf {
        named_bundle(root, BUNDLE_NAME, identifier, version)
    }

    #[test]
    fn only_the_expected_signature_identity_and_version_pass() {
        let root = temporary("verify");
        let app = bundle(&root, BUNDLE_ID, "9.9.9");
        // An ad-hoc signature satisfies a plain identifier requirement…
        verify_bundle(&app, "9.9.9", &plain(BUNDLE_ID)).unwrap();
        // …but never a team's.
        let error = verify_bundle(&app, "9.9.9", &requirement_for(Some("ABCDE12345"))).unwrap_err();
        assert!(error.to_string().contains("code signature"), "{error}");
        assert!(verify_bundle(&app, "9.9.8", &plain(BUNDLE_ID)).is_err());
        // Hex's identity is not Agent-Desk's.
        let old = bundle(&root.join("other"), "anomaly.hex", "9.9.9");
        assert!(verify_bundle(&old, "9.9.9", &plain("anomaly.hex")).is_err());
        // A modified bundle no longer satisfies its signature.
        fs::write(app.join("Contents/MacOS/agent-desk"), b"tampered").unwrap();
        assert!(verify_bundle(&app, "9.9.9", &plain(BUNDLE_ID)).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_swap_exchanges_bundles_and_launch_cleanup_removes_the_old_one() {
        let root = temporary("swap");
        let installed = root.join(BUNDLE_NAME);
        fs::create_dir_all(installed.join("Contents")).unwrap();
        fs::write(installed.join("Contents/version"), "old").unwrap();
        let staging = root.join(format!("{STAGING_PREFIX}9.9.9"));
        let update = staging.join(BUNDLE_NAME);
        fs::create_dir_all(update.join("Contents")).unwrap();
        fs::write(update.join("Contents/version"), "new").unwrap();

        swap(&Staged {
            update: update.clone(),
            installed: installed.clone(),
        })
        .unwrap();
        assert_eq!(
            fs::read_to_string(installed.join("Contents/version")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(update.join("Contents/version")).unwrap(),
            "old"
        );

        remove_staging(&root, STAGING_PREFIX);
        assert!(!staging.exists());
        assert!(installed.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_zip_unpacks_beside_the_bundle_and_replaces_older_staging() {
        let root = temporary("unpack");
        let app = bundle(&root.join("source"), BUNDLE_ID, "9.9.9");
        let archive = root.join("Agent-Desk.zip");
        let status = Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(&app)
            .arg(&archive)
            .status()
            .unwrap();
        assert!(status.success());
        let staging = root.join(format!("{STAGING_PREFIX}9.9.9"));
        fs::create_dir_all(staging.join("stale")).unwrap();
        unpack(&archive, &staging).unwrap();
        assert!(!staging.join("stale").exists());
        verify_bundle(&staging.join(BUNDLE_NAME), "9.9.9", &plain(BUNDLE_ID)).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn translocated_and_read_only_locations_are_not_replaceable() {
        let root = temporary("replaceable");
        let app = root.join(BUNDLE_NAME);
        fs::create_dir_all(&app).unwrap();
        assert!(replaceable(&app));
        assert!(!replaceable(
            &root.join("AppTranslocation/x/Agent-Desk.app")
        ));
        assert!(!replaceable(Path::new(
            "/System/Applications/Agent-Desk.app"
        )));
        let _ = fs::remove_dir_all(root);
    }

    /// `/Applications` as an old Hex's update leaves it: the bridge
    /// `Hex.app` carrying Agent-Desk, and the replaced Hex in staging.
    fn applications_after_hex_update(root: &Path, version: &str) -> PathBuf {
        let bridge = named_bundle(root, "Hex.app", "anomaly.hex", version);
        let library = bridge.join("Contents/Library");
        fs::create_dir_all(&library).unwrap();
        bundle(&library, BUNDLE_ID, version);
        let replaced = root.join(".Hex-update-9.9.9/Hex.app/Contents");
        fs::create_dir_all(&replaced).unwrap();
        bridge
    }

    #[test]
    fn the_bridge_installs_agent_desk_beside_it_once() {
        let root = temporary("bridge");
        let bridge = applications_after_hex_update(&root, "9.9.9");
        let carried = bridge.join("Contents/Library").join(BUNDLE_NAME);

        let installed = install_carried(&carried, &plain(BUNDLE_ID)).unwrap();
        assert_eq!(installed, root.join(BUNDLE_NAME));
        verify_bundle(&installed, "9.9.9", &plain(BUNDLE_ID)).unwrap();
        assert!(!root.join(format!("{STAGING_PREFIX}9.9.9")).exists());
        // Launching the bridge again keeps the installed bundle.
        fs::write(installed.join("Contents/marker"), b"kept").unwrap();
        install_carried(&carried, &plain(BUNDLE_ID)).unwrap();
        assert!(installed.join("Contents/marker").exists());

        // Agent-Desk's first launch removes the bridge and Hex's staging.
        remove_hex(&root);
        assert!(!bridge.exists());
        assert!(!root.join(".Hex-update-9.9.9").exists());
        assert!(installed.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_bridge_replaces_another_version_and_refuses_a_bad_bundle() {
        let root = temporary("bridge-replace");
        bundle(&root, BUNDLE_ID, "1.0.0");
        let bridge = applications_after_hex_update(&root, "9.9.9");
        let carried = bridge.join("Contents/Library").join(BUNDLE_NAME);
        let installed = install_carried(&carried, &plain(BUNDLE_ID)).unwrap();
        assert_eq!(
            plist_value(&installed, "CFBundleShortVersionString").unwrap(),
            "9.9.9"
        );
        assert!(!root.join(format!("{STAGING_PREFIX}9.9.9")).exists());

        // A carried bundle that fails its check changes nothing.
        let other = temporary("bridge-refuse");
        let bridge = applications_after_hex_update(&other, "9.9.9");
        let carried = bridge.join("Contents/Library").join(BUNDLE_NAME);
        fs::write(carried.join("Contents/MacOS/agent-desk"), b"tampered").unwrap();
        assert!(install_carried(&carried, &plain(BUNDLE_ID)).is_err());
        assert!(!other.join(BUNDLE_NAME).exists());
        assert!(bridge.exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(other);
    }

    #[test]
    fn only_a_bundle_with_hex_identity_is_removed() {
        let root = temporary("keep-other");
        let mine = named_bundle(&root, "Hex.app", "com.example.other", "1.0");
        remove_hex(&root);
        assert!(mine.exists(), "a different app named Hex stays");
        let _ = fs::remove_dir_all(root);
    }
}
