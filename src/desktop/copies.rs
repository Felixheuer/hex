//! Every copy of Hex this Mac knows about, and which of them are strays.
//!
//! macOS gives a permission (Accessibility, Screen Recording, …) to a bundle
//! id and its code signature. With a dozen built copies sharing `anomaly.hex`
//! (old `target/app` bundles, a download in ~/Downloads, Hex Test), it isn't
//! clear which copy a grant belongs to, and System Settings may list the
//! wrong one. Two copies are kept: **Live** (`/Applications/Hex.app`, the
//! Pilot release) and **Dev** (`/Applications/Hex Dev.app`, the one
//! development build). `scripts/install-fork.sh` removes the rest
//! (`hex dev copies --remove`), and the permission doctor names them.

use std::path::{Path, PathBuf};

/// Where Live is installed.
pub const LIVE: &str = "/Applications/Hex.app";
/// Where Dev is installed.
pub const DEV: &str = "/Applications/Hex Dev.app";

/// Bundle ids of Hex's apps (the old Kit-signed one included).
const FAMILY: &[&str] = &[
    "anomaly.hex",
    "anomaly.hex.dev",
    "anomaly.hex.test",
    "com.kitlangton.hex2",
];

/// One app bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Copy {
    pub path: PathBuf,
    pub bundle_id: String,
    /// `CFBundleShortVersionString`; a Pilot release's carries `-pilot.`.
    pub version: String,
}

impl Copy {
    /// Whether it is a published Pilot release rather than a local build.
    pub fn is_release(&self) -> bool {
        self.version.contains("-pilot.")
    }
}

/// What to do with a copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    /// Live or the Dev build being kept.
    Keep,
    /// Not a real copy of its own: a Gatekeeper translocation, a mounted
    /// disk image, or one already in the Trash.
    Ignore,
    /// Another copy that only confuses permissions: remove it.
    Remove,
}

/// What to do with `copy` when `keep` is the Dev build that stays (or
/// `None` when no Dev build is being installed).
pub fn verdict(copy: &Copy, keep: Option<&Path>) -> Verdict {
    let path = copy.path.to_string_lossy();
    if !FAMILY.contains(&copy.bundle_id.as_str()) || !path.ends_with(".app") {
        return Verdict::Ignore;
    }
    if path.contains("/AppTranslocation/")
        || path.starts_with("/Volumes/")
        || path.contains("/.Trash/")
    {
        return Verdict::Ignore;
    }
    if copy.path == Path::new(LIVE) || keep.is_some_and(|keep| copy.path == keep) {
        return Verdict::Keep;
    }
    // Inside another app (an embedded helper, a test fixture): not ours to
    // remove.
    let parent = copy
        .path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned());
    if parent.is_some_and(|parent| parent.contains(".app/")) {
        return Verdict::Ignore;
    }
    Verdict::Remove
}

/// The copies `verdict` removes, in the order given.
pub fn strays<'a>(copies: &'a [Copy], keep: Option<&Path>) -> Vec<&'a Copy> {
    copies
        .iter()
        .filter(|copy| verdict(copy, keep) == Verdict::Remove)
        .collect()
}

/// Live, when it is a local build rather than the Pilot release (it then
/// never updates itself).
pub fn local_live(copies: &[Copy]) -> Option<&Copy> {
    copies
        .iter()
        .find(|copy| copy.path == Path::new(LIVE) && !copy.is_release())
}

/// Every Hex bundle Spotlight knows, plus Live and Dev themselves.
#[cfg(target_os = "macos")]
pub fn find() -> Vec<Copy> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for id in FAMILY {
        let query = format!("kMDItemCFBundleIdentifier == '{id}'");
        if let Ok(output) = std::process::Command::new("/usr/bin/mdfind")
            .arg(query)
            .output()
        {
            paths.extend(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(PathBuf::from),
            );
        }
    }
    paths.extend([PathBuf::from(LIVE), PathBuf::from(DEV)]);
    paths.sort();
    paths.dedup();
    paths.into_iter().filter_map(|path| read(&path)).collect()
}

#[cfg(not(target_os = "macos"))]
pub fn find() -> Vec<Copy> {
    Vec::new()
}

/// `path`'s bundle id and version from its Info.plist.
#[cfg(target_os = "macos")]
fn read(path: &Path) -> Option<Copy> {
    let plist = path.join("Contents/Info.plist");
    let value = |key: &str| -> Option<String> {
        let output = std::process::Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(&plist)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    Some(Copy {
        path: path.to_path_buf(),
        bundle_id: value("CFBundleIdentifier")?,
        version: value("CFBundleShortVersionString").unwrap_or_default(),
    })
}

/// `hex dev copies`: lists every copy with what happens to it, and with
/// `remove` deletes the strays. `keep` is the Dev build to keep.
pub fn command(remove: bool, keep: Option<&Path>) -> color_eyre::Result<()> {
    let copies = find();
    for copy in &copies {
        let action = match verdict(copy, keep) {
            Verdict::Keep => "keep",
            Verdict::Ignore => "ignore",
            Verdict::Remove if remove => "remove",
            Verdict::Remove => "would remove",
        };
        println!(
            "{action:<12} {}  ({} {})",
            copy.path.display(),
            copy.bundle_id,
            copy.version
        );
    }
    if let Some(live) = local_live(&copies) {
        println!(
            "note         {} is a local build ({}), not the Pilot release: it never updates. \
             Install the release from https://ai.sms-pilot.de to make it Live.",
            live.path.display(),
            live.version
        );
    }
    if !remove {
        return Ok(());
    }
    let mut test_removed = false;
    for copy in strays(&copies, keep) {
        match std::fs::remove_dir_all(&copy.path) {
            Ok(()) => test_removed |= copy.bundle_id == "anomaly.hex.test",
            Err(error) => eprintln!("could not remove {}: {error}", copy.path.display()),
        }
    }
    // Hex Test is ad-hoc signed, so its grants never carry over to a new
    // build: drop the stale entries with the copies. Live's and Dev's stay.
    #[cfg(target_os = "macos")]
    if test_removed {
        let _ = std::process::Command::new("/usr/bin/tccutil")
            .args(["reset", "All", "anomaly.hex.test"])
            .output();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = test_removed;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(path: &str, id: &str, version: &str) -> Copy {
        Copy {
            path: path.into(),
            bundle_id: id.into(),
            version: version.into(),
        }
    }

    #[test]
    fn live_and_the_new_dev_build_stay_and_every_other_copy_goes() {
        let keep = Path::new(DEV);
        let copies = [
            copy(LIVE, "anomaly.hex", "3.4.0-pilot.6"),
            copy(DEV, "anomaly.hex.dev", "3.4.0"),
            copy(
                "/Users/p/dev/hex/target/app/Hex.app",
                "anomaly.hex",
                "3.4.0",
            ),
            copy(
                "/Users/p/dev/ai/target/app/Hex Test.app",
                "anomaly.hex.test",
                "3.4.0",
            ),
            copy(
                "/Users/p/dev/ai/target/app/Hex Dev.app",
                "anomaly.hex.dev",
                "3.4.0",
            ),
            copy("/Users/p/Downloads/Hex.app", "anomaly.hex", "3.4.0-pilot.2"),
            copy("/Applications/Hex Old.app", "com.kitlangton.hex2", "2.3.1"),
        ];
        let removed: Vec<&str> = strays(&copies, Some(keep))
            .iter()
            .map(|copy| copy.path.to_str().unwrap())
            .collect();
        assert_eq!(
            removed,
            [
                "/Users/p/dev/hex/target/app/Hex.app",
                "/Users/p/dev/ai/target/app/Hex Test.app",
                "/Users/p/dev/ai/target/app/Hex Dev.app",
                "/Users/p/Downloads/Hex.app",
                "/Applications/Hex Old.app",
            ]
        );
    }

    #[test]
    fn translocations_disk_images_trash_and_other_apps_are_left_alone() {
        let copies = [
            copy(
                "/private/var/folders/n0/T/AppTranslocation/9AE1/d/Hex.app",
                "anomaly.hex",
                "3.4.0-pilot.2",
            ),
            copy("/Volumes/Pilot/Hex.app", "anomaly.hex", "3.4.0-pilot.6"),
            copy("/Users/p/.Trash/Hex.app", "anomaly.hex", "3.4.0"),
            copy("/Applications/Other.app", "com.example.other", "1.0"),
            copy(
                "/Applications/Hex.app/Contents/Helpers/Hex Helper.app",
                "anomaly.hex",
                "3.4.0",
            ),
        ];
        assert!(strays(&copies, None).is_empty());
    }

    #[test]
    fn without_a_dev_build_being_installed_an_existing_dev_copy_is_a_stray() {
        let copies = [copy(DEV, "anomaly.hex.dev", "3.4.0")];
        assert_eq!(strays(&copies, None).len(), 1);
        assert!(strays(&copies, Some(Path::new(DEV))).is_empty());
    }

    #[test]
    fn a_local_build_in_the_live_place_is_named() {
        let release = [copy(LIVE, "anomaly.hex", "3.4.0-pilot.6")];
        assert!(local_live(&release).is_none());
        let local = [copy(LIVE, "anomaly.hex", "3.4.0")];
        assert_eq!(local_live(&local).unwrap().version, "3.4.0");
    }
}
