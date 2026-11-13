//! A hidden browser that plays the web's video: agent-browser's own Chrome
//! for Testing has no proprietary codecs (no H.264 or AAC), so most videos
//! don't play in it. When the person has a Chromium-family browser
//! installed, real Chrome sessions run that browser's executable instead
//! (agent-browser's `--executable-path`; its own profile folder, headless,
//! with a mock keychain, so the person's browser and profile are never
//! touched), else Chrome for Testing as before.
//!
//! Which apps qualify is read from their bundles, not from a list of
//! brands: an app that says it opens `https` links, embeds a framework with
//! Chromium's helper apps, and has a Chromium version (112 or later, for
//! the headless mode agent-browser uses). Chrome for Testing itself is
//! left out, being what this replaces. The person's default browser comes
//! first when it qualifies.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The first Chromium major version with the new headless mode.
const MIN_CHROMIUM: u32 = 112;

/// The executable for real Chrome sessions, when one qualifies (found once
/// per process; never in tests, which must not depend on this Mac).
pub fn chromium_executable() -> Option<PathBuf> {
    if cfg!(test) || !cfg!(target_os = "macos") {
        return None;
    }
    static FOUND: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let default =
                crate::assistant::actions::capabilities::launch_services().and_then(|services| {
                    crate::assistant::actions::capabilities::handler_for(&services, "https")
                });
            let mut roots = vec![PathBuf::from("/Applications")];
            if let Some(home) = crate::app_paths::home_dir() {
                roots.push(home.join("Applications"));
            }
            find_in(&roots, default.as_deref(), |app| {
                crate::assistant::actions::capabilities::info_plist(app)
            })
        })
        .clone()
}

/// The executable of the best app in `roots`: the default browser
/// `default` (a bundle id) first, then by name.
pub fn find_in(
    roots: &[PathBuf],
    default: Option<&str>,
    info: impl Fn(&Path) -> Option<Value>,
) -> Option<PathBuf> {
    let mut apps: Vec<PathBuf> = roots
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        // The bundle's structure first: cheap, and most apps fail it.
        .filter(|app| has_chromium_helpers(app))
        .collect();
    apps.sort();
    let mut found: Vec<(bool, PathBuf)> = apps
        .iter()
        .filter_map(|app| {
            let info = info(app)?;
            let executable = qualifies(app, &info)?;
            let is_default = default.is_some_and(|default| {
                info["CFBundleIdentifier"]
                    .as_str()
                    .is_some_and(|id| id.eq_ignore_ascii_case(default))
            });
            Some((is_default, executable))
        })
        .collect();
    found.sort_by_key(|(is_default, _)| !is_default);
    found.into_iter().next().map(|(_, executable)| executable)
}

/// Whether `app` embeds a framework with Chromium's helper apps
/// (`Contents/Frameworks/<Name> Framework.framework/…/Helpers/<Name> Helper.app`).
fn has_chromium_helpers(app: &Path) -> bool {
    let Ok(frameworks) = std::fs::read_dir(app.join("Contents/Frameworks")) else {
        return false;
    };
    frameworks.flatten().any(|framework| {
        let path = framework.path();
        path.extension()
            .is_some_and(|extension| extension == "framework")
            && [path.join("Helpers"), path.join("Versions/Current/Helpers")]
                .iter()
                .any(|helpers| {
                    std::fs::read_dir(helpers).is_ok_and(|entries| {
                        entries.flatten().any(|entry| {
                            entry.file_name().to_string_lossy().ends_with(" Helper.app")
                        })
                    })
                })
    })
}

/// `app`'s executable when its `info` (Info.plist) says it is a web
/// browser on a recent Chromium.
fn qualifies(app: &Path, info: &Value) -> Option<PathBuf> {
    let opens_web = info["CFBundleURLTypes"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|kind| kind["CFBundleURLSchemes"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .any(|scheme| scheme.eq_ignore_ascii_case("https"));
    let name = info["CFBundleName"].as_str().unwrap_or_default();
    let major: u32 = info["CFBundleShortVersionString"]
        .as_str()?
        .split('.')
        .next()?
        .parse()
        .ok()?;
    if !opens_web || major < MIN_CHROMIUM || name.contains("for Testing") {
        return None;
    }
    let executable = app
        .join("Contents/MacOS")
        .join(info["CFBundleExecutable"].as_str()?);
    crate::assistant::backends::search::is_executable(&executable).then_some(executable)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundle(root: &Path, name: &str, helper: bool) -> PathBuf {
        let app = root.join(format!("{name}.app"));
        let framework = app.join(format!(
            "Contents/Frameworks/{name} Framework.framework/Versions/A"
        ));
        std::fs::create_dir_all(&framework).unwrap();
        if helper {
            std::fs::create_dir_all(framework.join(format!("Helpers/{name} Helper.app"))).unwrap();
            let _ = std::os::unix::fs::symlink(
                "A",
                app.join(format!(
                    "Contents/Frameworks/{name} Framework.framework/Versions/Current"
                )),
            );
        }
        let macos = app.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let executable = macos.join(name);
        std::fs::write(&executable, b"#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        app
    }

    fn info(id: &str, name: &str, version: &str, https: bool) -> Value {
        let schemes = if https {
            json!(["http", "https"])
        } else {
            json!(["slack"])
        };
        json!({"CFBundleIdentifier": id, "CFBundleName": name, "CFBundleExecutable": name,
            "CFBundleShortVersionString": version, "CFBundleURLTypes": [{"CFBundleURLSchemes": schemes}]})
    }

    #[test]
    fn a_chromium_browser_is_found_by_its_bundle_and_the_default_comes_first() {
        let root = std::env::temp_dir().join(format!("hex-media-codecs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // A browser, a second one, an Electron app (no https), Chrome for
        // Testing, an old version, and an app without Chromium's helpers.
        bundle(&root, "Alpha Browser", true);
        bundle(&root, "Zeta Browser", true);
        bundle(&root, "Chat", true);
        bundle(&root, "Testing Browser", true);
        bundle(&root, "Old Browser", true);
        bundle(&root, "Plain", false);
        let plist = |app: &Path| -> Option<Value> {
            let name = app.file_stem()?.to_string_lossy().into_owned();
            Some(match name.as_str() {
                "Alpha Browser" => info("org.alpha", &name, "131.0.6778.86", true),
                "Zeta Browser" => info("org.zeta", &name, "142.1.2", true),
                "Chat" => info("org.chat", &name, "4.41.0", false),
                "Testing Browser" => info("org.cft", "Google Chrome for Testing", "157.0.1", true),
                "Old Browser" => info("org.old", &name, "1.9.3", true),
                _ => info("org.plain", &name, "200.0", true),
            })
        };
        let roots = [root.clone()];
        let first = find_in(&roots, None, plist).unwrap();
        assert!(
            first.ends_with("Alpha Browser.app/Contents/MacOS/Alpha Browser"),
            "{first:?}"
        );
        let default = find_in(&roots, Some("ORG.ZETA"), plist).unwrap();
        assert!(default.ends_with("Zeta Browser"), "{default:?}");
        // The default browser that isn't Chromium-based changes nothing.
        assert_eq!(find_in(&roots, Some("org.plain"), plist), Some(first));
        assert_eq!(find_in(&[root.join("none")], None, plist), None);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(chromium_executable(), None, "never this Mac's in tests");
    }
}
