//! The person keeps the front: an app a task opens, launches, reopens, or
//! brings back from another desktop may bring itself forward (TextEdit
//! opening a document does; Chrome making a window may), covering the
//! window the person types in and switching the menu bar to it. The guard
//! notes which app was in front before and, whenever the task's app took
//! the front from it (at once, or a moment later while it settles), gives
//! it back; the new window then sits behind, or parked off the screen once
//! `place_arrivals` moved it.

use std::time::{Duration, Instant};

/// How long after a launch, reopen, or recovery step an app may still
/// take the front by itself (Chrome restoring a window, an app finishing
/// launching): the guard keeps watching that long.
pub(super) const SETTLE: Duration = Duration::from_millis(if cfg!(test) { 0 } else { 1200 });

/// Which app is in front, and how the front goes back: this Mac's, or a
/// stand-in in tests (which never activate the person's apps).
pub trait Front: Send {
    /// The app in front now, by pid.
    fn frontmost(&mut self) -> Option<i32>;
    /// Brings app `pid` back to the front.
    fn give_back(&mut self, pid: i32);
}

/// This Mac's front.
pub struct System;

impl Front for System {
    fn frontmost(&mut self) -> Option<i32> {
        frontmost()
    }

    fn give_back(&mut self, pid: i32) {
        give_back(pid);
    }
}

/// Knows no front, so never gives one back (tests that don't look).
pub struct Unknown;

impl Front for Unknown {
    fn frontmost(&mut self) -> Option<i32> {
        None
    }

    fn give_back(&mut self, _pid: i32) {}
}

/// The app in front now, by pid.
#[cfg(target_os = "macos")]
pub(super) fn frontmost() -> Option<i32> {
    crate::dictation::context::frontmost_pid()
}

#[cfg(not(target_os = "macos"))]
pub(super) fn frontmost() -> Option<i32> {
    None
}

/// The app to give the front back to: the one in front `before`, when one
/// of the task's app's processes (`took`) is in front `now` instead. Not
/// when the person moved on to another app meanwhile, nor when the task's
/// app was in front already.
pub(super) fn give_back_to(before: Option<i32>, now: Option<i32>, took: &[i64]) -> Option<i32> {
    let (before, now) = (before?, now?);
    (now != before && took.contains(&i64::from(now))).then_some(before)
}

/// Brings the person's app (`pid`) back to the front, without waiting.
#[cfg(target_os = "macos")]
pub(super) fn give_back(pid: i32) {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    let Some(application) =
        autoreleasepool(|_| NSRunningApplication::runningApplicationWithProcessIdentifier(pid))
    else {
        return;
    };
    tracing::info!(
        pid,
        "a task's app took the front; giving it back to the person"
    );
    // ActivateIgnoringOtherApps is what raises an app over another one
    // from a background process.
    #[allow(deprecated)]
    let _ = autoreleasepool(|_| {
        application.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps)
    });
}

#[cfg(not(target_os = "macos"))]
pub(super) fn give_back(_pid: i32) {}

/// After something that may bring the app (`took`, its processes) forward:
/// gives the front back to `before` whenever the app took it, now and for
/// `settle` after; stops watching early once the person moved to another
/// app. Whether the front was given back.
pub(super) fn keep(
    front: &mut dyn Front,
    before: Option<i32>,
    took: &[i64],
    settle: Duration,
) -> bool {
    let started = Instant::now();
    let mut gave = false;
    loop {
        let now = front.frontmost();
        if let Some(pid) = give_back_to(before, now, took) {
            front.give_back(pid);
            gave = true;
        } else if now.is_some() && now != before {
            // The person is in another app now: theirs to keep.
            return gave;
        }
        if started.elapsed() >= settle {
            return gave;
        }
        std::thread::sleep(Duration::from_millis(100).min(settle));
    }
}

#[cfg(test)]
pub mod fake {
    use std::sync::{Arc, Mutex};

    /// A front the tests set; what was given back is kept.
    #[derive(Clone, Default)]
    pub struct Fake(Arc<Mutex<State>>);

    #[derive(Default)]
    pub struct State {
        pub front: Option<i32>,
        pub given: Vec<i32>,
    }

    impl Fake {
        pub fn at(pid: i32) -> Self {
            let fake = Self::default();
            fake.set(pid);
            fake
        }

        pub fn set(&self, pid: i32) {
            self.0.lock().unwrap().front = Some(pid);
        }

        pub fn now(&self) -> Option<i32> {
            self.0.lock().unwrap().front
        }

        pub fn given(&self) -> Vec<i32> {
            self.0.lock().unwrap().given.clone()
        }
    }

    impl super::Front for Fake {
        fn frontmost(&mut self) -> Option<i32> {
            self.now()
        }

        fn give_back(&mut self, pid: i32) {
            let mut state = self.0.lock().unwrap();
            state.front = Some(pid);
            state.given.push(pid);
        }
    }
}

/// Live, by hand only (`cargo test --bin agent-desk live_open_running_app
/// -- --ignored`): starts TextEdit in the background on a throwaway file,
/// then does what `hex_open_app` does to a running app (the `open -g`
/// reopen, recovery's new-window script) and keeps the front as the guard
/// does; the app in front must be the same before and after. Quits
/// TextEdit. Refuses when TextEdit already runs (the person's documents).
#[cfg(all(test, target_os = "macos"))]
mod live {
    use std::process::Command;
    use std::time::Duration;

    use super::super::super::screen::App;

    fn textedit_pids() -> Vec<i64> {
        String::from_utf8_lossy(
            &Command::new("/usr/bin/pgrep")
                .args(["-x", "TextEdit"])
                .output()
                .unwrap()
                .stdout,
        )
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .collect()
    }

    #[test]
    #[ignore = "drives TextEdit on this Mac; run by hand"]
    fn live_open_running_app() {
        assert!(
            textedit_pids().is_empty(),
            "TextEdit is open already: it may hold the person's documents"
        );
        struct Quit;
        impl Drop for Quit {
            fn drop(&mut self) {
                let _ = Command::new("/usr/bin/osascript")
                    .args([
                        "-e",
                        "tell application id \"com.apple.TextEdit\" to close every document saving no",
                        "-e",
                        "tell application id \"com.apple.TextEdit\" to quit saving no",
                    ])
                    .status();
            }
        }
        let _quit = Quit;
        let file = std::env::temp_dir().join(format!("agent-desk-live-{}.txt", std::process::id()));
        std::fs::write(&file, "throwaway").unwrap();
        let before = super::frontmost();
        println!("front before: {before:?}");
        Command::new("/usr/bin/open")
            .args(["-g", "-a", "TextEdit"])
            .arg(&file)
            .status()
            .unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let pids = textedit_pids();
        assert!(!pids.is_empty(), "TextEdit started");
        let after_start = super::frontmost();
        println!("front after a background start: {after_start:?}");
        let app = App {
            name: "TextEdit".into(),
            bundle_id: Some("com.apple.TextEdit".into()),
            path: "/System/Applications/TextEdit.app".into(),
        };
        // hex_open_app on the running app: the reopen it sends, then
        // recovery's new window, each followed by the guard's keep.
        let mut front = super::System;
        super::super::apps::reopen(&app);
        let gave_reopen = super::keep(&mut front, before, &pids, super::SETTLE);
        let after_reopen = super::frontmost();
        println!("front after reopen: {after_reopen:?} (given back: {gave_reopen})");
        let made = crate::assistant::actions::recovery::new_window(
            &crate::assistant::actions::recovery::new_window_script("com.apple.TextEdit"),
        );
        let gave_window = super::keep(&mut front, before, &pids, super::SETTLE);
        let after_window = super::frontmost();
        println!("front after a new window ({made}): {after_window:?} (given back: {gave_window})");
        let _ = std::fs::remove_file(&file);
        assert_eq!(after_reopen, before, "the person's app kept the front");
        assert_eq!(after_window, before, "the person's app kept the front");
        assert!(
            !after_window.is_some_and(|pid| pids.contains(&i64::from(pid))),
            "TextEdit isn't in front"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_goes_back_only_when_the_task_s_app_took_it() {
        // Mail was in front; TextEdit (launched, pid 77) took it.
        assert_eq!(give_back_to(Some(10), Some(77), &[77]), Some(10));
        // Any of the app's processes counts.
        assert_eq!(give_back_to(Some(10), Some(78), &[77, 78]), Some(10));
        // The launched app opened behind: nothing to do.
        assert_eq!(give_back_to(Some(10), Some(10), &[77]), None);
        // The person switched to Safari meanwhile: leave them there.
        assert_eq!(give_back_to(Some(10), Some(20), &[77]), None);
        // Unknown pids: nothing.
        assert_eq!(give_back_to(None, Some(77), &[77]), None);
        assert_eq!(give_back_to(Some(10), Some(77), &[]), None);
    }

    #[test]
    fn keeping_the_front_gives_it_back_and_leaves_the_person_where_they_went() {
        let mut front = fake::Fake::at(77);
        assert!(keep(&mut front, Some(10), &[77], Duration::ZERO));
        assert_eq!(front.now(), Some(10));
        assert_eq!(front.given(), [10]);
        // In front already, nothing taken: nothing given.
        let mut front = fake::Fake::at(10);
        assert!(!keep(&mut front, Some(10), &[77], Duration::ZERO));
        assert!(front.given().is_empty());
        // The person went to another app: theirs.
        let mut front = fake::Fake::at(20);
        assert!(!keep(&mut front, Some(10), &[77], Duration::from_secs(5)));
        assert!(front.given().is_empty());
    }
}
