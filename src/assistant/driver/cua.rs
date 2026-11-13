//! Cua Driver, installed by Hex: one pinned release from GitHub
//! (`trycua/cua`, tag `cua-driver-rs-v<VERSION>`), its archive checked
//! against the SHA-256 from that release's `SHA256SUMS` before it is
//! unpacked, and the app bundle checked as signed by Cua AI's Developer ID
//! (team `YCK386LBJ7`) and notarized. Nothing of its own installer runs:
//! that script connects coding agents, edits shell profiles, and runs a
//! self-test, none of which Hex wants. It lives in
//! `<support>/assistant/cua-driver/<VERSION>/`.
//!
//! Hex runs it with telemetry and update checks off and its state under
//! Hex's folder (`env`).

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, bail, eyre};

/// The pinned release.
pub const VERSION: &str = "0.49.1";

/// Cua AI, Inc.'s Apple Developer team.
const TEAM: &str = "YCK386LBJ7";

/// The archive for this Mac and its SHA-256 (from the release's
/// `SHA256SUMS`, read when the version was pinned).
fn archive() -> Option<(&'static str, &'static str)> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    Some(if cfg!(target_arch = "aarch64") {
        (
            "cua-driver-rs-0.49.1-darwin-arm64.tar.gz",
            "d7c6f8db05c5839f78a34673d846fed0910acda9ba29c14b0f0e3cb5e2adf58d",
        )
    } else {
        (
            "cua-driver-rs-0.49.1-darwin-x86_64.tar.gz",
            "47d74f598b9853b4008c1d616479075e557431a7bf096b46a0fbcbd7cde7cbaa",
        )
    })
}

fn url(file: &str) -> String {
    format!("https://github.com/trycua/cua/releases/download/cua-driver-rs-v{VERSION}/{file}")
}

/// Where the pinned release lives.
fn home() -> Option<PathBuf> {
    crate::assistant::store::assistant_dir()
        .ok()
        .map(|dir| dir.join("cua-driver"))
}

/// The executable inside an unpacked release folder.
fn executable_in(folder: &Path) -> PathBuf {
    folder.join("cua-driver")
}

/// The pinned Cua Driver, when installed.
pub fn installed() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let folder = home()?.join(VERSION);
    let found = std::fs::read_dir(&folder)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .map(|path| executable_in(&path))
        .find(|path| path.is_file())?;
    Some(found)
}

/// The environment Hex runs it with: no telemetry, no update checks, its
/// state in Hex's folder.
pub fn env() -> Vec<(String, String)> {
    let mut env = vec![
        ("DO_NOT_TRACK".to_owned(), "1".to_owned()),
        ("CUA_TELEMETRY_ENABLED".into(), "0".into()),
        ("CUA_DRIVER_RS_TELEMETRY_ENABLED".into(), "0".into()),
        ("CUA_DRIVER_RS_UPDATE_CHECK".into(), "0".into()),
        ("CUA_DRIVER_NO_CONNECT".into(), "1".into()),
    ];
    if let Some(home) = home() {
        env.push((
            "CUA_DRIVER_RS_HOME".into(),
            home.join("state").display().to_string(),
        ));
    }
    env
}

/// Installs the pinned release (a no-op when it is); its executable.
pub fn install(progress: &mut dyn FnMut(&str)) -> Result<PathBuf> {
    if let Some(path) = installed() {
        return Ok(path);
    }
    let (file, sha256) = archive().ok_or_else(|| eyre!("Cua Driver runs on macOS only"))?;
    let home = home().ok_or_else(|| eyre!("no application support folder"))?;
    let folder = home.join(VERSION);
    std::fs::create_dir_all(&home)?;
    let partial = home.join(format!("{file}.partial"));
    progress(&format!("Downloading Cua Driver {VERSION}…"));
    let address = url(file);
    crate::net::download(
        &crate::net::Download {
            url: &address,
            user_agent: &format!("hex/{}", crate::VERSION),
            size: None,
            canceled: None,
        },
        &partial,
        |_| {},
    )?;
    let digest = crate::transcription::models::sha256_hex(&partial, None)?;
    if digest != sha256 {
        let _ = std::fs::remove_file(&partial);
        bail!("Cua Driver's download didn't match its checksum; nothing was installed");
    }
    progress("Unpacking Cua Driver…");
    let staging = home.join(format!("{VERSION}.unpacking"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let unpacked = std::process::Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(&partial)
        .arg("-C")
        .arg(&staging)
        .status()?;
    if !unpacked.success() {
        bail!("Cua Driver's archive didn't unpack");
    }
    let release = std::fs::read_dir(&staging)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| executable_in(path).is_file())
        .ok_or_else(|| eyre!("Cua Driver's archive has no cua-driver"))?;
    verify_signature(&release.join("CuaDriver.app"))?;
    verify_signature(&executable_in(&release))?;
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::rename(&staging, &folder)?;
    let _ = std::fs::remove_file(&partial);
    let path = installed().ok_or_else(|| eyre!("Cua Driver didn't install"))?;
    tracing::info!(path = %path.display(), version = VERSION, "installed Cua Driver");
    Ok(path)
}

/// Whether `path` is signed by Cua AI's Developer ID (and valid).
fn verify_signature(path: &Path) -> Result<()> {
    let valid = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict"])
        .arg(path)
        .status()?;
    if !valid.success() {
        bail!("{} isn't validly signed", path.display());
    }
    let details = std::process::Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=2"])
        .arg(path)
        .output()?;
    let text = String::from_utf8_lossy(&details.stderr);
    if !signed_by_team(&text) {
        bail!("{} isn't signed by Cua AI", path.display());
    }
    Ok(())
}

/// Whether `codesign -dv` output names Cua AI's team.
fn signed_by_team(codesign: &str) -> bool {
    codesign
        .lines()
        .any(|line| line.trim() == format!("TeamIdentifier={TEAM}"))
        && codesign
            .lines()
            .any(|line| line.starts_with("Authority=Developer ID Application: Cua AI"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn the_release_is_pinned_and_checked() {
        let (file, sha) = archive().unwrap();
        assert!(file.contains(VERSION));
        assert_eq!(sha.len(), 64);
        assert!(url(file).starts_with("https://github.com/trycua/cua/releases/download/"));
        assert!(url(file).ends_with(file));
        let names = env();
        assert!(names.iter().any(|(key, value)| key == "DO_NOT_TRACK" && value == "1"));
        assert!(
            names
                .iter()
                .any(|(key, value)| key == "CUA_DRIVER_RS_UPDATE_CHECK" && value == "0")
        );
    }

    #[test]
    fn only_cua_ai_s_signature_counts() {
        let theirs = "Identifier=com.trycua.driver\nAuthority=Developer ID Application: Cua AI, Inc. (YCK386LBJ7)\nTeamIdentifier=YCK386LBJ7\n";
        assert!(signed_by_team(theirs));
        let other = "Authority=Developer ID Application: Someone (ABCDE12345)\nTeamIdentifier=ABCDE12345\n";
        assert!(!signed_by_team(other));
    }
}
