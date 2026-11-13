//! The update against a fake release feed and fake releases: real archives
//! (tar) of a fake `cua-driver` (the guard's fake Cua Driver, or one that
//! won't start), signatures and notarization read from marker files, and the
//! real self-test (`guard::driver_self_test`) on what was unpacked.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

const TEAM: &str = cua::TEAM;

/// A folder of its own under the system's temporary folder.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "agent-desk-cua-update-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// How a fake release is made.
#[derive(Clone, Copy)]
struct Make {
    team: &'static str,
    notarized: bool,
    /// Its driver starts and answers (else it exits at once).
    works: bool,
}

const GOOD: Make = Make {
    team: TEAM,
    notarized: true,
    works: true,
};

/// The network and the signature checks, faked.
#[derive(Default)]
struct Fake {
    texts: Mutex<HashMap<String, String>>,
    files: Mutex<HashMap<String, PathBuf>>,
    downloads: Mutex<Vec<String>>,
}

impl Checks for Fake {
    fn download(&self, url: &str, to: &Path) -> Result<()> {
        self.downloads.lock().unwrap().push(url.to_owned());
        let from = self
            .files
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| eyre!("404 {url}"))?;
        std::fs::copy(from, to)?;
        Ok(())
    }

    fn text(&self, url: &str) -> Result<String> {
        self.texts
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| eyre!("404 {url}"))
    }

    fn unpack(&self, archive: &Path, into: &Path) -> Result<()> {
        cua::System.unpack(archive, into)
    }

    fn team(&self, path: &Path) -> Result<String> {
        let marker = path.parent().unwrap().join("TEAM");
        let team = std::fs::read_to_string(marker).map_err(|_| eyre!("not signed"))?;
        Ok(team.trim().to_owned())
    }

    fn notarized(&self, app: &Path) -> Result<()> {
        if app.join("NOTARIZED").is_file() {
            Ok(())
        } else {
            bail!("{} isn't notarized", app.display())
        }
    }
}

fn fake_driver() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/assistant/guard/fake-cua-driver.py")
}

impl Fake {
    /// Publishes `version`: its archive, its checksum lists (with `sha` in
    /// place of the real one when given), and itself in the feed.
    fn publish(&self, scratch: &Scratch, version: &str, make: Make, sha: Option<&str>) {
        let name = cua::archive_name(version).unwrap();
        let folder = scratch.0.join(format!("build-{version}"));
        let release = folder.join(name.trim_end_matches(".tar.gz"));
        std::fs::create_dir_all(release.join("CuaDriver.app")).unwrap();
        let script = if make.works {
            format!(
                "#!/bin/sh\nexec /usr/bin/python3 -I '{}' \"$@\"\n",
                fake_driver().display()
            )
        } else {
            "#!/bin/sh\nexit 3\n".to_owned()
        };
        let executable = release.join("cua-driver");
        std::fs::write(&executable, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(release.join("TEAM"), make.team).unwrap();
        if make.notarized {
            std::fs::write(release.join("CuaDriver.app/NOTARIZED"), "").unwrap();
        }
        let archive = scratch.0.join(&name);
        let packed = std::process::Command::new("/usr/bin/tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&folder)
            .arg(release.file_name().unwrap())
            .status()
            .unwrap();
        assert!(packed.success());
        let real = crate::transcription::models::sha256_hex(&archive, None).unwrap();
        let sums = format!("{}  {name}\n", sha.unwrap_or(&real));
        let mut texts = self.texts.lock().unwrap();
        texts.insert(cua::url(version, "SHA256SUMS"), sums.clone());
        texts.insert(
            cua::url(version, "checksums.txt"),
            format!("## SHA256 Checksums\n```\n{sums}```\n"),
        );
        self.files
            .lock()
            .unwrap()
            .insert(cua::url(version, &name), archive);
        let mut feed: Vec<Value> = texts
            .get(FEED)
            .map(|feed| serde_json::from_str(feed).unwrap())
            .unwrap_or_default();
        feed.insert(
            0,
            json!({
                "tag_name": format!("cua-driver-rs-v{version}"), "draft": false, "prerelease": true,
                "assets": [
                    {"name": name, "digest": format!("sha256:{real}")},
                    {"name": "SHA256SUMS"}, {"name": "checksums.txt"},
                ],
            }),
        );
        texts.insert(FEED.into(), serde_json::to_string(&feed).unwrap());
    }
}

use serde_json::json;

/// Installs `version` into `home` as the current one, as a first install
/// would have.
fn installed(home: &Home, fake: &Fake, scratch: &Scratch, version: &str) {
    fake.publish(scratch, version, GOOD, None);
    let name = cua::archive_name(version).unwrap();
    let sha =
        cua::checksum_in(&fake.text(&cua::url(version, "SHA256SUMS")).unwrap(), &name).unwrap();
    cua::stage(home, fake, version, &name, &sha, TEAM, &mut |_| {}).unwrap();
    home.switch_to(version).unwrap();
}

fn update<'a>(home: &'a Home, fake: &'a Fake, idle: &'a dyn Fn() -> bool) -> Update<'a> {
    Update {
        home,
        checks: fake,
        self_test: &crate::assistant::guard::driver_self_test,
        idle,
        idle_wait: Duration::ZERO,
        now_ms: 1_000,
    }
}

#[test]
fn the_feed_gives_the_newest_stable_driver_for_this_mac() {
    let name = |version: &str| cua::archive_name(version).unwrap();
    let release = |tag: &str, version: &str, extra: Value| {
        let mut item = json!({"tag_name": tag, "draft": false, "prerelease": true,
            "assets": [{"name": name(version), "digest": "sha256:ABC"}, {"name": "SHA256SUMS"}]});
        if let Some(fields) = extra.as_object() {
            for (key, value) in fields {
                item[key] = value.clone();
            }
        }
        item
    };
    let feed = json!([
        release(
            "nightly-cua-driver-rs-v0.50.1-nightly.20261113.1",
            "0.50.1-nightly.20261113.1",
            json!({})
        ),
        release("cua-driver-rs-v0.49.9", "0.49.9", json!({})),
        release("cua-driver-rs-v0.49.10", "0.49.10", json!({})),
        release("cua-driver-rs-v0.51.0", "0.51.0", json!({"draft": true})),
        {"tag_name": "cua-driver-rs-v0.52.0", "draft": false, "assets": [{"name": name("0.52.0")}]},
        {"tag_name": "cua-computer-server-v9.0.0", "draft": false, "assets": []},
    ]);
    let found = latest(&feed.to_string()).unwrap().unwrap();
    // Nightlies, drafts, and a release without a checksum list don't count;
    // versions compare as numbers.
    assert_eq!(found.version, "0.49.10");
    assert_eq!(found.archive, name("0.49.10"));
    assert_eq!(found.digest.as_deref(), Some("abc"));
    assert_eq!(found.sums, ["SHA256SUMS"]);
    assert!(newer("0.49.10", "0.49.9"));
    assert!(!newer("0.49.1", "0.49.1"));
    assert!(latest("not json").is_err());
}

#[test]
fn a_newer_release_is_checked_tested_and_switched_to() {
    let scratch = Scratch::new();
    let home = Home(scratch.0.join("cua-driver"));
    let fake = Fake::default();
    installed(&home, &fake, &scratch, "0.49.1");
    let idle = || true;
    // Nothing newer: up to date, nothing downloaded.
    let before = fake.downloads.lock().unwrap().len();
    let status = run(&update(&home, &fake, &idle), &mut |_| {});
    assert_eq!(status.outcome, Outcome::UpToDate, "{status:?}");
    assert_eq!(fake.downloads.lock().unwrap().len(), before);
    assert_eq!(status.line(), "Cua Driver 0.49.1 · up to date");
    assert!(!status.due(1_000 + 60_000));
    assert!(status.due(1_000 + EVERY.as_millis() as u64));

    fake.publish(&scratch, "0.49.3", GOOD, None);
    let status = run(&update(&home, &fake, &idle), &mut |_| {});
    assert_eq!(status.outcome, Outcome::Updated, "{status:?}");
    assert_eq!(home.current().as_deref(), Some("0.49.3"));
    assert_eq!(home.previous().as_deref(), Some("0.49.1"));
    assert_eq!(status.line(), "Cua Driver 0.49.3 · updated from 0.49.1");
    assert_eq!(status_in(&home), status, "kept in update.json");

    // The next one: the oldest goes, the previous stays for a rollback.
    fake.publish(&scratch, "0.50.0", GOOD, None);
    let status = run(&update(&home, &fake, &idle), &mut |_| {});
    assert_eq!(status.outcome, Outcome::Updated, "{status:?}");
    assert!(home.executable("0.49.1").is_none(), "pruned");
    assert_eq!(home.previous().as_deref(), Some("0.49.3"));
    assert_eq!(home.roll_back().unwrap(), "0.49.3");
    assert_eq!(home.current().as_deref(), Some("0.49.3"));
    assert_eq!(home.previous().as_deref(), Some("0.50.0"));
}

#[test]
fn a_release_that_fails_a_check_is_never_switched_to() {
    let idle = || true;
    let cases: [(&str, Make, Option<&str>, &str); 4] = [
        (
            "checksum",
            GOOD,
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            "disagree",
        ),
        (
            "team",
            Make {
                team: "ABCDE12345",
                ..GOOD
            },
            None,
            "not Cua AI's",
        ),
        (
            "notarization",
            Make {
                notarized: false,
                ..GOOD
            },
            None,
            "notarized",
        ),
        (
            "self-test",
            Make {
                works: false,
                ..GOOD
            },
            None,
            "self-test",
        ),
    ];
    for (what, make, sha, says) in cases {
        let scratch = Scratch::new();
        let home = Home(scratch.0.join("cua-driver"));
        let fake = Fake::default();
        installed(&home, &fake, &scratch, "0.49.1");
        fake.publish(&scratch, "0.49.3", make, sha);
        let status = run(&update(&home, &fake, &idle), &mut |_| {});
        assert_eq!(status.outcome, Outcome::Failed, "{what}: {status:?}");
        assert!(
            status.error.as_deref().unwrap_or_default().contains(says),
            "{what}: {status:?}"
        );
        assert_eq!(home.current().as_deref(), Some("0.49.1"), "{what}");
        assert!(
            home.executable("0.49.3").is_none(),
            "{what}: nothing staged"
        );
        assert_eq!(
            status.line(),
            "Cua Driver 0.49.1 · update failed (kept 0.49.1)"
        );
    }
}

#[test]
fn the_switch_waits_until_no_task_acts() {
    let scratch = Scratch::new();
    let home = Home(scratch.0.join("cua-driver"));
    let fake = Fake::default();
    installed(&home, &fake, &scratch, "0.49.1");
    fake.publish(&scratch, "0.49.3", GOOD, None);
    let busy = || false;
    let status = run(&update(&home, &fake, &busy), &mut |_| {});
    assert_eq!(status.outcome, Outcome::Waiting, "{status:?}");
    assert_eq!(
        home.current().as_deref(),
        Some("0.49.1"),
        "still the old one"
    );
    assert!(status.due(2_000), "tried again soon");
    assert!(
        status
            .line()
            .contains("0.49.3 is ready once no task is acting")
    );
    // Once idle, the staged and tested release is switched to without a
    // second download.
    let downloads = fake.downloads.lock().unwrap().len();
    let idle = || true;
    let status = run(&update(&home, &fake, &idle), &mut |_| {});
    assert_eq!(status.outcome, Outcome::Updated, "{status:?}");
    assert_eq!(home.current().as_deref(), Some("0.49.3"));
    assert_eq!(fake.downloads.lock().unwrap().len(), downloads);
}

#[test]
fn a_driver_that_fails_where_tasks_run_it_is_rolled_back() {
    let scratch = Scratch::new();
    let home = Home(scratch.0.join("cua-driver"));
    let fake = Fake::default();
    installed(&home, &fake, &scratch, "0.49.1");
    fake.publish(&scratch, "0.49.3", GOOD, None);
    // It passes beside the current one, then fails after the switch.
    let tests = AtomicU32::new(0);
    let self_test = |path: &Path| -> std::result::Result<(), String> {
        if tests.fetch_add(1, Ordering::SeqCst) == 0 {
            crate::assistant::guard::driver_self_test(path)
        } else {
            Err("it stopped".into())
        }
    };
    let idle = || true;
    let status = run(
        &Update {
            self_test: &self_test,
            ..update(&home, &fake, &idle)
        },
        &mut |_| {},
    );
    assert_eq!(status.outcome, Outcome::Failed, "{status:?}");
    assert!(status.error.as_deref().unwrap().contains("back on 0.49.1"));
    assert_eq!(home.current().as_deref(), Some("0.49.1"));
    assert_eq!(status.installed.as_deref(), Some("0.49.1"));
}

#[test]
fn the_current_version_switches_in_one_rename_and_rolls_back() {
    let scratch = Scratch::new();
    let home = Home(scratch.0.join("cua-driver"));
    let fake = Fake::default();
    assert_eq!(home.current(), None);
    assert!(home.roll_back().is_err());
    installed(&home, &fake, &scratch, "0.49.1");
    assert!(home.switch_to("0.99.0").is_err(), "not installed");
    // Written as `current`, never half.
    assert_eq!(
        std::fs::read_to_string(home.path().join("current")).unwrap(),
        "0.49.1\n"
    );
    // A folder from before the updater (no `current`) is the pinned one.
    std::fs::remove_file(home.path().join("current")).unwrap();
    let pinned = Home(scratch.0.join("pinned"));
    std::fs::create_dir_all(pinned.path()).unwrap();
    std::fs::rename(home.path().join("0.49.1"), pinned.path().join(cua::PINNED)).unwrap();
    assert_eq!(pinned.current().as_deref(), Some(cua::PINNED));
    // A leftover install is cleaned up.
    std::fs::create_dir_all(pinned.path().join("0.50.0.unpacking")).unwrap();
    pinned.prune();
    assert!(!pinned.path().join("0.50.0.unpacking").exists());
    assert_eq!(pinned.current().as_deref(), Some(cua::PINNED));
}

#[test]
fn the_fake_driver_passes_the_self_test_and_a_broken_one_fails_it() {
    let scratch = Scratch::new();
    let good = scratch.0.join("good");
    std::fs::write(
        &good,
        format!(
            "#!/bin/sh\nexec /usr/bin/python3 -I '{}' \"$@\"\n",
            fake_driver().display()
        ),
    )
    .unwrap();
    let broken = scratch.0.join("broken");
    std::fs::write(&broken, "#!/bin/sh\nexit 3\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    for path in [&good, &broken] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    crate::assistant::guard::driver_self_test(&good).unwrap();
    assert!(crate::assistant::guard::driver_self_test(&broken).is_err());
}
