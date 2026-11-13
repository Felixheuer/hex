//! Cua Driver on this Mac: its releases from GitHub (`trycua/cua`, tags
//! `cua-driver-rs-v<version>`), each archive checked against the SHA-256 its
//! release publishes before it is unpacked, and the app bundle and the
//! executable checked as signed by Cua AI's Developer ID (the team of the
//! installed, trusted driver; [`TEAM`] before there is one) and notarized.
//! Nothing of its own installer runs: that script connects coding agents,
//! edits shell profiles, and runs a self-test, none of which Agent-Desk wants.
//!
//! Its folder, `<support>/assistant/cua-driver/`:
//!
//! - `<version>/<release>/cua-driver`: one folder per installed version;
//! - `current`: the version tasks run (written atomically, so a switch is
//!   one rename); absent, the [`PINNED`] one when it is there;
//! - `previous`: the version before it, kept for a rollback;
//! - `update.json`: the last check and how it went (`update`).
//!
//! Agent-Desk runs it with telemetry and update checks off and its state under
//! Agent-Desk's folder ([`env`]).

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, bail, eyre};

/// The release a first install takes, with its checksums compiled in;
/// `update` moves on from there.
pub const PINNED: &str = "0.49.1";

/// Cua AI, Inc.'s Apple Developer team, trusted until an installed driver
/// says it (`trusted_team`).
pub const TEAM: &str = "YCK386LBJ7";

/// The pinned archive for this Mac and its SHA-256 (from the release's
/// `SHA256SUMS`, read when the version was pinned).
fn pinned_archive() -> Option<(&'static str, &'static str)> {
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

/// The archive name of `version` for this Mac.
pub fn archive_name(version: &str) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    Some(format!("cua-driver-rs-{version}-darwin-{arch}.tar.gz"))
}

/// Where a release's files are downloaded from.
pub fn url(version: &str, file: &str) -> String {
    format!("https://github.com/trycua/cua/releases/download/cua-driver-rs-v{version}/{file}")
}

/// Cua Driver's folder.
fn home() -> Option<PathBuf> {
    crate::assistant::store::assistant_dir()
        .ok()
        .map(|dir| dir.join("cua-driver"))
}

/// The standard folder ([`Home`]), when there is an application support
/// folder.
pub fn standard() -> Option<Home> {
    home().map(Home)
}

/// Cua Driver's folder and what is installed in it.
#[derive(Clone, Debug)]
pub struct Home(pub PathBuf);

impl Home {
    pub fn path(&self) -> &Path {
        &self.0
    }

    fn read(&self, name: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.0.join(name)).ok()?;
        let version = text.trim();
        (!version.is_empty() && valid_version(version)).then(|| version.to_owned())
    }

    /// Writes `name` as one rename: a reader sees the old version or the
    /// new one, never half of either.
    fn write(&self, name: &str, version: &str) -> Result<()> {
        std::fs::create_dir_all(&self.0)?;
        let partial = self.0.join(format!(".{name}.partial"));
        std::fs::write(&partial, format!("{version}\n"))?;
        std::fs::rename(&partial, self.0.join(name))?;
        Ok(())
    }

    /// The version tasks run.
    pub fn current(&self) -> Option<String> {
        self.read("current")
            .filter(|version| self.executable(version).is_some())
            .or_else(|| self.executable(PINNED).map(|_| PINNED.to_owned()))
    }

    /// The version kept for a rollback.
    pub fn previous(&self) -> Option<String> {
        self.read("previous")
            .filter(|version| self.executable(version).is_some())
    }

    /// The executable of installed `version`.
    pub fn executable(&self, version: &str) -> Option<PathBuf> {
        std::fs::read_dir(self.0.join(version))
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .map(|path| executable_in(&path))
            .find(|path| path.is_file())
    }

    /// The current version's executable.
    pub fn installed(&self) -> Option<PathBuf> {
        self.executable(&self.current()?)
    }

    /// Makes `version` (installed) the one tasks run; the one before it
    /// becomes the previous.
    pub fn switch_to(&self, version: &str) -> Result<()> {
        if self.executable(version).is_none() {
            bail!("Cua Driver {version} isn't installed");
        }
        let before = self.current();
        if let Some(before) = before.as_deref().filter(|before| *before != version) {
            self.write("previous", before)?;
        }
        self.write("current", version)
    }

    /// Goes back to the previous version; the one it leaves becomes the
    /// previous (a second rollback undoes the first).
    pub fn roll_back(&self) -> Result<String> {
        let previous = self
            .previous()
            .ok_or_else(|| eyre!("there's no earlier Cua Driver to go back to"))?;
        self.switch_to(&previous)?;
        Ok(previous)
    }

    /// Removes every version but the current and the previous one, and
    /// what an interrupted install left.
    pub fn prune(&self) {
        let keep = [self.current(), self.previous()];
        let Ok(entries) = std::fs::read_dir(&self.0) else {
            return;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let leftover = name.ends_with(".unpacking") || name.ends_with(".partial");
            let old = path.is_dir()
                && valid_version(&name)
                && !keep.iter().flatten().any(|kept| *kept == name);
            if leftover || old {
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
            }
        }
    }
}

/// `1.2.3` (digits and dots, three parts at most four), so a version is a
/// folder name and nothing else.
pub fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    (3..=4).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 6 && part.bytes().all(|b| b.is_ascii_digit())
        })
}

/// The executable inside an unpacked release folder.
fn executable_in(folder: &Path) -> PathBuf {
    folder.join("cua-driver")
}

/// The current Cua Driver, when installed.
pub fn installed() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    standard()?.installed()
}

/// The current version, when installed.
pub fn version() -> Option<String> {
    if cfg!(test) {
        return None;
    }
    standard()?.current()
}

/// The environment Agent-Desk runs it with: no telemetry, no update checks of
/// its own, its state in Agent-Desk's folder.
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

/// What a release's files are checked with, and how they are fetched: the
/// real ones ([`System`]), or fakes in tests.
pub trait Checks {
    /// Downloads `url` to `to`.
    fn download(&self, url: &str, to: &Path) -> Result<()>;
    /// A small text file (a checksum list, the release feed).
    fn text(&self, url: &str) -> Result<String>;
    /// Unpacks the `.tar.gz` at `archive` into `into`.
    fn unpack(&self, archive: &Path, into: &Path) -> Result<()>;
    /// The Developer ID team that validly signed `path`; an error when it
    /// isn't validly signed by a Developer ID.
    fn team(&self, path: &Path) -> Result<String>;
    /// Whether the app bundle at `app` is notarized (Gatekeeper accepts it
    /// as a notarized Developer ID app).
    fn notarized(&self, app: &Path) -> Result<()>;
}

/// The real checks: HTTPS through `crate::net`, `/usr/bin/tar`,
/// `codesign`, and `spctl`.
pub struct System;

impl Checks for System {
    fn download(&self, url: &str, to: &Path) -> Result<()> {
        crate::net::download(
            &crate::net::Download {
                url,
                user_agent: &user_agent(),
                size: None,
                canceled: None,
            },
            to,
            |_| {},
        )
        .map(|_| ())
    }

    fn text(&self, url: &str) -> Result<String> {
        use std::io::Read;
        const MAX: u64 = 8 * 1024 * 1024;
        let agent = crate::net::agent(url, |config| {
            config
                .https_only(true)
                .max_redirects(10)
                .http_status_as_error(false)
                .timeout_global(Some(std::time::Duration::from_secs(60)))
        });
        let mut response = agent
            .get(url)
            .header("User-Agent", user_agent().as_str())
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|error| eyre!("{}", crate::net::unreachable(url, &error)))?;
        let status = response.status().as_u16();
        if status != 200 {
            bail!("{} answered HTTP {status}", crate::net::host(url));
        }
        let mut text = String::new();
        response
            .body_mut()
            .with_config()
            .limit(MAX + 1)
            .reader()
            .take(MAX + 1)
            .read_to_string(&mut text)?;
        if text.len() as u64 > MAX {
            bail!("{} sent too much", crate::net::host(url));
        }
        Ok(text)
    }

    fn unpack(&self, archive: &Path, into: &Path) -> Result<()> {
        let unpacked = std::process::Command::new("/usr/bin/tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(into)
            .status()?;
        if !unpacked.success() {
            bail!("Cua Driver's archive didn't unpack");
        }
        Ok(())
    }

    fn team(&self, path: &Path) -> Result<String> {
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
        let team = developer_id_team(&String::from_utf8_lossy(&details.stderr))
            .ok_or_else(|| eyre!("{} isn't signed with a Developer ID", path.display()))?;
        // The text above is the signature's own word: prove it with a
        // certificate Apple issued for that team, or a self-made one named
        // like it would pass.
        let anchored = std::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(developer_id_requirement(&team)?)
            .arg(path)
            .status()?;
        if !anchored.success() {
            bail!(
                "{} isn't signed with Apple's Developer ID certificate for team {team}",
                path.display()
            );
        }
        Ok(team)
    }

    fn notarized(&self, app: &Path) -> Result<()> {
        let assessed = std::process::Command::new("/usr/sbin/spctl")
            .args(["--assess", "--type", "execute", "-vv"])
            .arg(app)
            .output()?;
        let text = String::from_utf8_lossy(&assessed.stderr);
        if !assessed.status.success() || !notarized_in(&text) {
            bail!("{} isn't notarized", app.display());
        }
        Ok(())
    }
}

fn user_agent() -> String {
    format!("agent-desk/{}", crate::VERSION)
}

/// `codesign -R`'s requirement that a signature chains to Apple through a
/// Developer ID Application certificate issued to `team`.
pub fn developer_id_requirement(team: &str) -> Result<String> {
    if team.len() != 10 || !team.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("“{team}” isn't a team id");
    }
    Ok(format!(
        "-R=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists \
         and certificate leaf[field.1.2.840.113635.100.6.1.13] exists \
         and certificate leaf[subject.OU] = \"{team}\""
    ))
}

/// The team of a Developer ID signature in `codesign -dv` output.
pub fn developer_id_team(codesign: &str) -> Option<String> {
    let team = codesign
        .lines()
        .find_map(|line| line.trim().strip_prefix("TeamIdentifier="))?
        .trim()
        .to_owned();
    let developer_id = codesign.lines().any(|line| {
        line.trim()
            .strip_prefix("Authority=Developer ID Application: ")
            .is_some_and(|who| who.trim_end().ends_with(&format!("({team})")))
    });
    (developer_id && !team.is_empty() && team != "not set").then_some(team)
}

/// Whether `spctl -a -vv` output says notarized.
fn notarized_in(spctl: &str) -> bool {
    spctl
        .lines()
        .any(|line| line.trim() == "source=Notarized Developer ID")
}

/// The team a new release must be signed by: the installed driver's, read
/// from its signature (it was checked when it came), else [`TEAM`].
pub fn trusted_team(home: &Home, checks: &dyn Checks) -> String {
    home.installed()
        .and_then(|path| checks.team(&path).ok())
        .unwrap_or_else(|| TEAM.to_owned())
}

/// The SHA-256 of `file` in a checksum list (`<hex>  <name>` lines, as in
/// `SHA256SUMS` or `checksums.txt`; anything else is skipped).
pub fn checksum_in(list: &str, file: &str) -> Option<String> {
    list.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == file && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// Downloads `version`'s `file`, checks it against `sha256`, unpacks it,
/// checks its signatures (by `team`) and notarization, and puts it in
/// `<home>/<version>/` (an earlier copy there is replaced). Doesn't make it
/// current. Its executable.
pub fn stage(
    home: &Home,
    checks: &dyn Checks,
    version: &str,
    file: &str,
    sha256: &str,
    team: &str,
    progress: &mut dyn FnMut(&str),
) -> Result<PathBuf> {
    if !valid_version(version) {
        bail!("“{version}” isn't a Cua Driver version");
    }
    std::fs::create_dir_all(home.path())?;
    let partial = home.path().join(format!("{file}.partial"));
    progress(&format!("Downloading Cua Driver {version}…"));
    let _ = std::fs::remove_file(&partial);
    checks.download(&url(version, file), &partial)?;
    let digest = crate::transcription::models::sha256_hex(&partial, None)?;
    if !digest.eq_ignore_ascii_case(sha256) {
        let _ = std::fs::remove_file(&partial);
        bail!("Cua Driver {version}'s download didn't match its checksum; nothing was installed");
    }
    progress(&format!("Checking Cua Driver {version}…"));
    let staging = home.path().join(format!("{version}.unpacking"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let checked = (|| -> Result<()> {
        checks.unpack(&partial, &staging)?;
        let release = std::fs::read_dir(&staging)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| executable_in(path).is_file())
            .ok_or_else(|| eyre!("Cua Driver's archive has no cua-driver"))?;
        let app = release.join("CuaDriver.app");
        for path in [&app, &executable_in(&release)] {
            let signed_by = checks.team(path)?;
            if signed_by != team {
                bail!(
                    "{} is signed by team {signed_by}, not Cua AI's ({team})",
                    path.display()
                );
            }
        }
        checks.notarized(&app)
    })();
    let _ = std::fs::remove_file(&partial);
    if let Err(error) = checked {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    let folder = home.path().join(version);
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::rename(&staging, &folder)?;
    home.executable(version)
        .ok_or_else(|| eyre!("Cua Driver {version} didn't install"))
}

/// Installs the pinned release when no Cua Driver is (a no-op when one
/// is); the current executable.
pub fn install(progress: &mut dyn FnMut(&str)) -> Result<PathBuf> {
    if let Some(path) = installed() {
        return Ok(path);
    }
    let home = standard().ok_or_else(|| eyre!("no application support folder"))?;
    install_pinned(&home, &System, progress)
}

/// [`install`] into `home` with `checks`.
pub fn install_pinned(
    home: &Home,
    checks: &dyn Checks,
    progress: &mut dyn FnMut(&str),
) -> Result<PathBuf> {
    if let Some(path) = home.installed() {
        return Ok(path);
    }
    let (file, sha256) = pinned_archive().ok_or_else(|| eyre!("Cua Driver runs on macOS only"))?;
    let path = stage(home, checks, PINNED, file, sha256, TEAM, progress)?;
    home.switch_to(PINNED)?;
    tracing::info!(path = %path.display(), version = PINNED, "installed Cua Driver");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn the_first_install_is_pinned_and_checked() {
        let (file, sha) = pinned_archive().unwrap();
        assert!(file.contains(PINNED));
        assert_eq!(Some(file.to_owned()), archive_name(PINNED));
        assert_eq!(sha.len(), 64);
        assert!(url(PINNED, file).starts_with("https://github.com/trycua/cua/releases/download/"));
        assert!(url(PINNED, file).ends_with(file));
        let names = env();
        assert!(
            names
                .iter()
                .any(|(key, value)| key == "DO_NOT_TRACK" && value == "1")
        );
        assert!(
            names
                .iter()
                .any(|(key, value)| key == "CUA_DRIVER_RS_UPDATE_CHECK" && value == "0")
        );
    }

    #[test]
    fn only_a_developer_id_signature_names_a_team() {
        let theirs = "Identifier=com.trycua.driver\nAuthority=Developer ID Application: Cua AI, Inc. (YCK386LBJ7)\nAuthority=Developer ID Certification Authority\nTeamIdentifier=YCK386LBJ7\n";
        assert_eq!(developer_id_team(theirs).as_deref(), Some("YCK386LBJ7"));
        let requirement = developer_id_requirement("YCK386LBJ7").unwrap();
        assert!(requirement.starts_with("-R=anchor apple generic and certificate 1["));
        assert!(requirement.ends_with("certificate leaf[subject.OU] = \"YCK386LBJ7\""));
        assert!(developer_id_requirement("YCK\" or 1").is_err());
        let other =
            "Authority=Developer ID Application: Someone (ABCDE12345)\nTeamIdentifier=ABCDE12345\n";
        assert_eq!(developer_id_team(other).as_deref(), Some("ABCDE12345"));
        // An Apple Development (not Developer ID) or ad-hoc signature.
        let development =
            "Authority=Apple Development: me (ABCDE12345)\nTeamIdentifier=ABCDE12345\n";
        assert_eq!(developer_id_team(development), None);
        assert_eq!(
            developer_id_team("Signature=adhoc\nTeamIdentifier=not set\n"),
            None
        );
        assert!(notarized_in(
            "x: accepted\nsource=Notarized Developer ID\norigin=…"
        ));
        assert!(!notarized_in("x: accepted\nsource=Developer ID\n"));
    }

    #[test]
    fn checksums_are_read_from_either_list() {
        let sums = "# Cua Driver\n```\n4e4a92c36bbe83a62037b08caeec00495a71088be23fceb34d6209cdf9e9f9a5  cua-driver-rs-0.49.3-darwin-arm64.tar.gz\nd0a026fd83cf4f9dd97c1cbaec9743cb16a9b98cc74dd28e855b639758e15a5a *cua-driver-rs-0.49.3-darwin-x86_64.tar.gz\n```\n";
        assert_eq!(
            checksum_in(sums, "cua-driver-rs-0.49.3-darwin-arm64.tar.gz").as_deref(),
            Some("4e4a92c36bbe83a62037b08caeec00495a71088be23fceb34d6209cdf9e9f9a5")
        );
        assert!(checksum_in(sums, "cua-driver-rs-0.49.3-darwin-x86_64.tar.gz").is_some());
        assert_eq!(
            checksum_in(sums, "cua-driver-rs-0.49.3-darwin.tar.gz"),
            None
        );
        assert_eq!(checksum_in("abc  file\n", "file"), None);
    }

    #[test]
    fn versions_are_folder_names_and_nothing_else() {
        assert!(valid_version("0.49.3"));
        assert!(valid_version("1.0.0.2"));
        assert!(!valid_version("0.49"));
        assert!(!valid_version("../0.49.3"));
        assert!(!valid_version("0.48.1-nightly.20261113"));
        assert!(!valid_version(""));
    }
}
