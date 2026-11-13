//! The programs watching a video needs, managed like Hex's other helpers:
//!
//! - yt-dlp: a pinned release binary ([`YT_DLP_VERSION`]), downloaded once
//!   into Hex's own tools folder and checked against the SHA-256 its release
//!   publishes before it may run (and again once per process). The person's
//!   own yt-dlp on the agent PATH is the fallback when the download fails.
//! - ffmpeg: Hex's own copy in the tools folder when there is one, else the
//!   one on the agent PATH (Homebrew's), else `brew install ffmpeg`.
//!
//! Nothing here prompts the person: a download Hex writes itself carries no
//! quarantine flag, and Homebrew needs no password.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::assistant::backends::{run, search};

/// The pinned yt-dlp release.
pub const YT_DLP_VERSION: &str = "2026.10.26";

/// One release binary: its asset name, size, and SHA-256 (from the
/// release's `SHA2-256SUMS`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Build {
    pub asset: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// This system's yt-dlp build; `None` where the project publishes none.
pub fn yt_dlp_build() -> Option<Build> {
    let (asset, size, sha256) = if cfg!(target_os = "macos") {
        (
            "yt-dlp_macos",
            37_466_304,
            "0f922c15ccc0b83eefa1c8bc9104e434318c4bf16e408e1ee93b7745a6efe0aa",
        )
    } else if cfg!(windows) {
        (
            "yt-dlp.exe",
            23_575_744,
            "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d",
        )
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        (
            "yt-dlp_linux_aarch64",
            40_789_792,
            "e87e9533e89fe5889abaa21f5114dfe7756d6aa6219b83551add04d48b18e483",
        )
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (
            "yt-dlp_linux",
            41_166_968,
            "767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795",
        )
    } else {
        return None;
    };
    Some(Build {
        asset,
        size,
        sha256,
    })
}

fn release_url(build: &Build) -> String {
    format!(
        "https://github.com/yt-dlp/yt-dlp/releases/download/{YT_DLP_VERSION}/{}",
        build.asset
    )
}

/// Hex's own tools folder.
pub fn tools_dir() -> Option<PathBuf> {
    let dir = crate::app_paths::support_dir().ok()?.join("tools");
    crate::app_paths::create_private_dir(&dir).ok()?;
    Some(dir)
}

fn program(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// Where the pinned yt-dlp lives in `tools`.
pub fn yt_dlp_path(tools: &Path) -> PathBuf {
    tools
        .join(format!("yt-dlp-{YT_DLP_VERSION}"))
        .join(program("yt-dlp"))
}

/// `bytes`' SHA-256 as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether the file at `path` is exactly `build`.
pub fn verified(path: &Path, build: &Build) -> bool {
    std::fs::read(path)
        .is_ok_and(|bytes| bytes.len() as u64 == build.size && sha256_hex(&bytes) == build.sha256)
}

/// The pinned yt-dlp, downloaded and checked first when it isn't there;
/// the person's own yt-dlp when that fails. `progress` hears what happens.
pub fn yt_dlp(progress: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    static CHECKED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(path) = CHECKED.get() {
        return Ok(path.clone());
    }
    let managed = match (tools_dir(), yt_dlp_build()) {
        (Some(tools), Some(build)) => {
            let path = yt_dlp_path(&tools);
            if verified(&path, &build) {
                Ok(path)
            } else {
                progress("Installing the video downloader (yt-dlp)");
                install(&path, &build)
            }
        }
        _ => Err("yt-dlp has no build for this system".to_owned()),
    };
    let found = managed.or_else(|why| {
        search::find("yt-dlp").ok_or_else(|| format!("Hex couldn't install yt-dlp: {why}"))
    })?;
    let _ = CHECKED.set(found.clone());
    Ok(found)
}

/// Downloads `build` to `path`, keeping it only when its checksum matches.
fn install(path: &Path, build: &Build) -> Result<PathBuf, String> {
    let dir = path.parent().ok_or("no folder for yt-dlp")?;
    crate::app_paths::create_private_dir(dir).map_err(|error| error.to_string())?;
    let partial = path.with_extension("partial");
    let _ = std::fs::remove_file(&partial);
    let url = release_url(build);
    crate::net::download(
        &crate::net::Download {
            url: &url,
            user_agent: "Hex",
            size: Some(build.size),
            canceled: None,
        },
        &partial,
        |_| {},
    )
    .map_err(|error| format!("{error:#}"))?;
    if !verified(&partial, build) {
        let _ = std::fs::remove_file(&partial);
        return Err(format!(
            "the download of {} didn't match its published checksum; nothing was installed",
            build.asset
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
    }
    std::fs::rename(&partial, path).map_err(|error| error.to_string())?;
    Ok(path.to_owned())
}

/// ffmpeg: Hex's own, the agent PATH's, or installed with Homebrew.
pub fn ffmpeg(progress: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    if let Some(found) = find_ffmpeg() {
        return Ok(found);
    }
    let brew = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .map(PathBuf::from)
        .into_iter()
        .find(|path| search::is_executable(path))
        .or_else(|| search::find("brew"))
        .ok_or("ffmpeg isn't installed and there is no Homebrew to install it with; the person can install ffmpeg (https://ffmpeg.org/download.html)")?;
    progress("Installing ffmpeg with Homebrew");
    let mut command = search::command(&brew);
    command.args(["install", "ffmpeg"]);
    let ran = run::bounded(command, Duration::from_secs(420)).map_err(|error| error.to_string())?;
    if !ran.success() {
        return Err(format!(
            "`brew install ffmpeg` failed: {}",
            ran.last_words()
        ));
    }
    search::forget();
    find_ffmpeg().ok_or_else(|| "ffmpeg isn't on the PATH after installing it".to_owned())
}

fn find_ffmpeg() -> Option<PathBuf> {
    tools_dir()
        .map(|tools| tools.join("ffmpeg").join(program("ffmpeg")))
        .filter(|path| search::is_executable(path))
        .or_else(|| search::find("ffmpeg"))
}

/// What `hex dev assistant-doctor` prints: which yt-dlp and ffmpeg a video
/// would be watched with (nothing is installed).
pub fn status() -> String {
    let yt_dlp = match (tools_dir(), yt_dlp_build()) {
        (Some(tools), Some(build)) if verified(&yt_dlp_path(&tools), &build) => {
            format!(
                "yt-dlp {YT_DLP_VERSION} (checked) at {}",
                yt_dlp_path(&tools).display()
            )
        }
        (_, Some(_)) => match search::find("yt-dlp") {
            Some(path) => format!(
                "yt-dlp {YT_DLP_VERSION} not installed yet (first video installs it); the PATH has {}",
                path.display()
            ),
            None => format!("yt-dlp {YT_DLP_VERSION} not installed yet (first video installs it)"),
        },
        (_, None) => "yt-dlp has no build for this system".to_owned(),
    };
    let ffmpeg = find_ffmpeg().map_or_else(
        || "ffmpeg missing (first video installs it with Homebrew)".to_owned(),
        |path| format!("ffmpeg at {}", path.display()),
    );
    format!("{yt_dlp}; {ffmpeg}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Installs the pinned yt-dlp (into HEX_APPLICATION_SUPPORT_DIR) and reads
    /// one public video's facts without downloading it:
    /// `cargo test --bin hex media::tools::tests::yt_dlp_installs_and_reads -- --ignored --nocapture`.
    #[test]
    #[ignore = "downloads yt-dlp"]
    fn yt_dlp_installs_and_reads() {
        let started = std::time::Instant::now();
        let path = yt_dlp(&mut |step| println!("{step}")).unwrap();
        println!(
            "yt-dlp at {} in {:.1} s",
            path.display(),
            started.elapsed().as_secs_f64()
        );
        let url = std::env::var("HEX_MEDIA_PROBE_URL")
            .unwrap_or_else(|_| "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into());
        let mut command = std::process::Command::new(&path);
        command
            .args([
                "-J",
                "--skip-download",
                "--no-playlist",
                "--ignore-config",
                "--",
            ])
            .arg(&url);
        let started = std::time::Instant::now();
        let ran = run::bounded(command, Duration::from_secs(90)).unwrap();
        let info: serde_json::Value = serde_json::from_str(&ran.stdout).unwrap_or_default();
        println!(
            "probe in {:.1} s: {:?} by {:?}, {:?} s; {}",
            started.elapsed().as_secs_f64(),
            info["title"],
            info["uploader"],
            info["duration"],
            ran.last_words()
        );
        assert!(ran.success() || !ran.stderr.is_empty());
    }

    #[test]
    fn the_pinned_build_is_checked_by_size_and_checksum() {
        let build = yt_dlp_build().expect("a build for the systems Hex runs on");
        assert_eq!(build.sha256.len(), 64);
        assert!(release_url(&build).contains(&format!("/download/{YT_DLP_VERSION}/")));
        let dir = std::env::temp_dir().join(format!("hex-media-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("yt-dlp");
        std::fs::write(&file, b"not yt-dlp").unwrap();
        assert!(!verified(&file, &build));
        let fake = Build {
            asset: "fake",
            size: 10,
            sha256: "0aa94c87fbf26c5bd8f06ade3c8a95deefc8acde2b8a6c8b1a5ee1e0a0f3a4b0",
        };
        assert!(
            !verified(&file, &fake),
            "the size matches, the checksum doesn't"
        );
        let right = Build {
            sha256: Box::leak(sha256_hex(b"not yt-dlp").into_boxed_str()),
            ..fake
        };
        assert!(verified(&file, &right));
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert!(yt_dlp_path(Path::new("/x")).starts_with(format!("/x/yt-dlp-{YT_DLP_VERSION}")));
    }
}
