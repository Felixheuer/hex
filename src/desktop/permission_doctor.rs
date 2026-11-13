//! Finds why macOS permissions can't apply to this copy of Agent-Desk, before
//! onboarding or a permission row sends the person to System Settings:
//!
//! - **Translocated:** a downloaded `Agent-Desk.app` still carries
//!   `com.apple.quarantine`, so Gatekeeper runs a read-only copy from a
//!   random `…/AppTranslocation/…` path. System Settings never lists it and
//!   grants never reach it. Repair: drop the quarantine from the original
//!   bundle, open that, and quit this copy.
//! - **Not installed:** this copy runs from outside /Applications (a build
//!   folder, Downloads), so a grant may land on another copy.
//! - **Other copies:** more bundles with this bundle id exist (old builds,
//!   downloads), so it isn't clear which one a grant belongs to. Repair:
//!   remove them (`desktop::copies`), keeping Live and Dev.
//!
//! The decisions are pure ([`diagnose_with`]); [`diagnose`] reads this Mac.

use std::path::{Path, PathBuf};

use super::copies::{self, Copy};

/// What stands between Agent-Desk and its permissions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Finding {
    /// Running from a Gatekeeper translocation; `original` is the bundle it
    /// was copied from, when macOS says.
    Translocated { original: Option<PathBuf> },
    /// Running from outside /Applications.
    NotInstalled { running: PathBuf },
    /// Other bundles with this bundle id that only confuse grants.
    OtherCopies(Vec<PathBuf>),
}

/// The first problem for an Agent-Desk running from `running` (its `.app`), with
/// `own_id` and `copies` the bundles this Mac knows; `None` when grants
/// can apply. `original` is a translocation's source, when known.
pub fn diagnose_with(
    running: &Path,
    own_id: &str,
    original: Option<&Path>,
    copies: &[Copy],
) -> Option<Finding> {
    let path = running.to_string_lossy();
    if !path.ends_with(".app") {
        // A loose binary (cargo run, tests): nothing to say.
        return None;
    }
    if path.contains("/AppTranslocation/") {
        return Some(Finding::Translocated {
            original: original.map(Path::to_path_buf),
        });
    }
    if !running.starts_with("/Applications") {
        return Some(Finding::NotInstalled {
            running: running.to_path_buf(),
        });
    }
    let others: Vec<PathBuf> = copies::strays(copies, Some(running))
        .into_iter()
        .filter(|copy| copy.bundle_id == own_id)
        .map(|copy| copy.path.clone())
        .collect();
    (!others.is_empty()).then_some(Finding::OtherCopies(others))
}

/// What a check says for each finding (the setup page translates them;
/// the paths go to the log).
pub const TRANSLOCATED: &str = "macOS runs a protected copy of Agent-Desk because it was downloaded, so System Settings can't give it permissions";
pub const NOT_INSTALLED: &str =
    "Agent-Desk runs from outside Applications, so permissions may go to another copy";
pub const OTHER_COPIES: &str =
    "Other copies of Agent-Desk share its permissions, so System Settings may show the wrong one";
pub const NEEDS_RESTART: &str = "Screen Recording is on, but Agent-Desk must restart to use it";

/// What the person reads for `finding`.
pub fn message(finding: &Finding) -> &'static str {
    match finding {
        Finding::Translocated { .. } => TRANSLOCATED,
        Finding::NotInstalled { .. } => NOT_INSTALLED,
        Finding::OtherCopies(_) => OTHER_COPIES,
    }
}

/// [`diagnose_with`] for the running Agent-Desk on this Mac.
#[cfg(target_os = "macos")]
pub fn diagnose() -> Option<Finding> {
    let running = running_bundle()?;
    let own_id = super::permission_guide::running_bundle_id();
    let original = translocation_original(&running);
    let copies = if running.to_string_lossy().contains("/AppTranslocation/") {
        Vec::new()
    } else {
        copies::find()
    };
    diagnose_with(&running, &own_id, original.as_deref(), &copies)
}

#[cfg(not(target_os = "macos"))]
pub fn diagnose() -> Option<Finding> {
    None
}

/// The `.app` this process runs from.
#[cfg(target_os = "macos")]
fn running_bundle() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    executable
        .ancestors()
        .nth(3)
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(Path::to_path_buf)
}

/// The bundle a translocated `path` was copied from
/// (`SecTranslocateCreateOriginalPathForURL`).
#[cfg(target_os = "macos")]
fn translocation_original(path: &Path) -> Option<PathBuf> {
    use objc2_foundation::{NSString, NSURL};

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecTranslocateCreateOriginalPathForURL(
            translocated: *const std::ffi::c_void,
            error: *mut *const std::ffi::c_void,
        ) -> *const std::ffi::c_void;
    }
    if !path.to_string_lossy().contains("/AppTranslocation/") {
        return None;
    }
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: NSURL is toll-free bridged to CFURL; the returned CFURL is
        // owned (Create rule) and released through `Retained`.
        let original = unsafe {
            SecTranslocateCreateOriginalPathForURL(
                (&*url as *const NSURL).cast(),
                std::ptr::null_mut(),
            )
        };
        if original.is_null() {
            return None;
        }
        // SAFETY: a +1 CFURL, which is an NSURL.
        let original: objc2::rc::Retained<NSURL> =
            unsafe { objc2::rc::Retained::from_raw(original.cast_mut().cast())? };
        original.path().map(|path| PathBuf::from(path.to_string()))
    })
}

/// Repairs `finding` where Agent-Desk can: a translocation (quarantine dropped,
/// the original opened once this copy quits) or other copies (removed).
/// Returns whether Agent-Desk quits for it.
#[cfg(target_os = "macos")]
pub fn repair(finding: &Finding) -> color_eyre::Result<bool> {
    use color_eyre::eyre::{bail, eyre};
    match finding {
        Finding::Translocated { original } => {
            let original = original
                .clone()
                .or_else(|| Some(PathBuf::from(copies::LIVE)).filter(|path| path.exists()))
                .ok_or_else(|| eyre!("macOS didn't say where Agent-Desk was copied from"))?;
            let status = std::process::Command::new("/usr/bin/xattr")
                .args(["-dr", "com.apple.quarantine"])
                .arg(&original)
                .status()?;
            if !status.success() {
                bail!(
                    "couldn't remove the download mark from {}",
                    original.display()
                );
            }
            if !super::onboarding::spawn_open_after_exit(&original) {
                bail!("couldn't schedule opening {}", original.display());
            }
            crate::macos_rune::quit();
            Ok(true)
        }
        Finding::OtherCopies(others) => {
            for path in others {
                match std::fs::remove_dir_all(path) {
                    // Gone already (a release build moved on): nothing to do.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => bail!("couldn't remove {}: {error}", path.display()),
                    Ok(()) => {}
                }
                forget_registration(path);
            }
            Ok(false)
        }
        Finding::NotInstalled { .. } => {
            bail!("Move Agent-Desk to Applications in Finder, then open it from there")
        }
    }
}

/// Tells LaunchServices to forget the bundle at `path`, so a removed copy
/// stops counting as one of Agent-Desk's.
#[cfg(target_os = "macos")]
fn forget_registration(path: &Path) {
    const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    if let Err(error) = std::process::Command::new(LSREGISTER)
        .arg("-u")
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        tracing::warn!(%error, path = %path.display(), "couldn't unregister a removed copy");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(path: &str, id: &str) -> Copy {
        Copy {
            path: path.into(),
            bundle_id: id.into(),
            version: "3.4.0".into(),
        }
    }

    #[test]
    fn a_translocated_copy_is_found_first() {
        let running = Path::new("/private/var/folders/n0/T/AppTranslocation/9AE1/d/Agent-Desk.app");
        assert_eq!(
            diagnose_with(
                running,
                "fheuer.agentdesk",
                Some(Path::new(copies::LIVE)),
                &[]
            ),
            Some(Finding::Translocated {
                original: Some(copies::LIVE.into())
            })
        );
    }

    #[test]
    fn a_copy_outside_applications_is_named() {
        let running = Path::new("/Users/p/Downloads/Agent-Desk.app");
        assert!(matches!(
            diagnose_with(running, "fheuer.agentdesk", None, &[]),
            Some(Finding::NotInstalled { .. })
        ));
    }

    #[test]
    fn other_copies_of_the_same_app_are_listed_and_live_and_dev_stay() {
        let copies = [
            copy(copies::LIVE, "fheuer.agentdesk"),
            copy(copies::DEV, "fheuer.agentdesk.dev"),
            copy(
                "/Users/p/dev/hex/target/app/Agent-Desk.app",
                "fheuer.agentdesk",
            ),
            copy(
                "/Users/p/dev/hex/target/app/Agent-Desk Test.app",
                "fheuer.agentdesk.test",
            ),
        ];
        assert_eq!(
            diagnose_with(Path::new(copies::LIVE), "fheuer.agentdesk", None, &copies),
            Some(Finding::OtherCopies(vec![
                "/Users/p/dev/hex/target/app/Agent-Desk.app".into()
            ]))
        );
        // Dev has no other copy of its own id.
        assert_eq!(
            diagnose_with(
                Path::new(copies::DEV),
                "fheuer.agentdesk.dev",
                None,
                &copies
            ),
            None
        );
    }

    #[test]
    fn a_loose_binary_and_a_clean_install_have_nothing_to_fix() {
        assert_eq!(
            diagnose_with(
                Path::new("/Users/p/dev/hex/target/debug/agent-desk"),
                "fheuer.agentdesk",
                None,
                &[]
            ),
            None
        );
        assert_eq!(
            diagnose_with(
                Path::new(copies::LIVE),
                "fheuer.agentdesk",
                None,
                &[copy(copies::LIVE, "fheuer.agentdesk")]
            ),
            None
        );
    }
}
