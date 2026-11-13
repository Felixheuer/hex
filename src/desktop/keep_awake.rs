//! Keeps the computer awake while the assistant's background jobs work.
//!
//! A job may run for hours or days; if the Mac goes to sleep it stops. No
//! mouse movement: while a Background job is queued, running, taken over,
//! or waiting out a usage limit, Agent-Desk holds a power assertion
//! (`PreventUserIdleSystemSleep` on macOS, so the display may still sleep;
//! with `keep_display_awake` also `PreventUserIdleDisplaySleep`), and lets
//! it go when none is left. A job waiting for the person's answer doesn't
//! keep the computer up. Closing a laptop's lid on battery still sleeps it:
//! no assertion prevents that.
//!
//! macOS uses IOKit assertions, Windows `SetThreadExecutionState`; elsewhere
//! this does nothing.

use std::sync::Mutex;

/// What to hold.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Want {
    #[default]
    Nothing,
    /// The computer stays awake; the display may sleep.
    System,
    /// Both stay awake.
    SystemAndDisplay,
}

/// What `jobs` working background jobs need, as the settings allow. Jobs
/// that may operate the desktop (`desktop`) keep the display awake too: a
/// sleeping display stops their clicks while the person is away.
pub fn want_for(keep_awake: bool, keep_display_awake: bool, desktop: bool, jobs: usize) -> Want {
    want(keep_awake, keep_display_awake || desktop, jobs)
}

/// What `jobs` working background jobs need, as the settings allow.
pub fn want(keep_awake: bool, keep_display_awake: bool, jobs: usize) -> Want {
    match (keep_awake && jobs > 0, keep_display_awake) {
        (false, _) => Want::Nothing,
        (true, false) => Want::System,
        (true, true) => Want::SystemAndDisplay,
    }
}

struct Held {
    want: Want,
    jobs: usize,
    handle: platform::Handle,
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// Holds or lets go as `jobs` working background jobs need. Cheap to call
/// on every change: nothing happens while the need stays the same.
pub fn update(keep_awake: bool, keep_display_awake: bool, jobs: usize) {
    let desktop = crate::app_settings::assistant_runtime()
        .is_some_and(|(on, settings)| on && settings.control_desktop);
    update_in(&HELD, keep_awake, keep_display_awake, desktop, jobs);
}

// Off macOS the platform's handle is `()`.
#[cfg_attr(not(target_os = "macos"), allow(clippy::unit_arg))]
fn update_in(
    state: &Mutex<Option<Held>>,
    keep_awake: bool,
    keep_display_awake: bool,
    desktop: bool,
    jobs: usize,
) {
    let next = want_for(keep_awake, keep_display_awake, desktop, jobs);
    let mut held = state.lock().unwrap_or_else(|error| error.into_inner());
    let current = held.as_ref().map_or(Want::Nothing, |held| held.want);
    let same_count = held.as_ref().is_some_and(|held| held.jobs == jobs);
    if current == next && (next == Want::Nothing || same_count) {
        return;
    }
    if let Some(old) = held.take() {
        platform::release(old.handle);
    }
    if next == Want::Nothing {
        tracing::info!("no background jobs: the computer may sleep again");
        return;
    }
    let reason = if jobs == 1 {
        "Agent-Desk: 1 assistant task is running".to_owned()
    } else {
        format!("Agent-Desk: {jobs} assistant tasks are running")
    };
    match platform::hold(next, &reason) {
        Some(handle) => {
            if current == Want::Nothing {
                tracing::info!(
                    jobs,
                    ?next,
                    "keeping the computer awake for background jobs"
                );
            }
            *held = Some(Held {
                want: next,
                jobs,
                handle,
            });
        }
        None => tracing::warn!("could not keep the computer awake"),
    }
}

#[cfg(all(target_os = "macos", not(test)))]
mod platform {
    use objc2_foundation::NSString;

    use super::Want;

    /// The assertion ids held.
    pub type Handle = Vec<u32>;

    const LEVEL_ON: u32 = 255;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: *const std::ffi::c_void,
            level: u32,
            name: *const std::ffi::c_void,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    fn create(kind: &str, reason: &str) -> Option<u32> {
        let kind = NSString::from_str(kind);
        let reason = NSString::from_str(reason);
        let mut id = 0u32;
        // SAFETY: NSString is toll-free bridged to CFString; both outlive
        // the call, and `id` is a valid out pointer.
        let status = unsafe {
            IOPMAssertionCreateWithName(
                (&*kind as *const NSString).cast(),
                LEVEL_ON,
                (&*reason as *const NSString).cast(),
                &mut id,
            )
        };
        (status == 0).then_some(id)
    }

    pub fn hold(want: Want, reason: &str) -> Option<Handle> {
        let mut ids = vec![create("PreventUserIdleSystemSleep", reason)?];
        if want == Want::SystemAndDisplay {
            ids.extend(create("PreventUserIdleDisplaySleep", reason));
        }
        Some(ids)
    }

    pub fn release(handle: Handle) {
        for id in handle {
            // SAFETY: `id` came from IOPMAssertionCreateWithName.
            unsafe { IOPMAssertionRelease(id) };
        }
    }
}

#[cfg(all(target_os = "windows", not(test)))]
mod platform {
    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    use super::Want;

    /// The state is per thread; Agent-Desk sets it from whichever thread updates,
    /// and `release` clears it on that same call path.
    pub type Handle = ();

    pub fn hold(want: Want, _reason: &str) -> Option<Handle> {
        let mut flags = ES_CONTINUOUS | ES_SYSTEM_REQUIRED;
        if want == Want::SystemAndDisplay {
            flags |= ES_DISPLAY_REQUIRED;
        }
        // SAFETY: plain flags.
        (unsafe { SetThreadExecutionState(flags) } != 0).then_some(())
    }

    pub fn release(_handle: Handle) {
        // SAFETY: plain flags.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "windows"))))]
mod platform {
    use super::Want;

    pub type Handle = Want;

    pub fn hold(want: Want, _reason: &str) -> Option<Handle> {
        Some(want)
    }

    pub fn release(_handle: Handle) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_computer_stays_awake_only_while_jobs_work_and_the_setting_allows() {
        assert_eq!(want(true, false, 0), Want::Nothing);
        assert_eq!(want(true, false, 1), Want::System);
        assert_eq!(want(true, true, 3), Want::SystemAndDisplay);
        assert_eq!(want(false, true, 3), Want::Nothing, "turned off");
    }

    #[test]
    fn jobs_that_may_use_the_desktop_keep_the_display_awake() {
        assert_eq!(want_for(true, false, true, 1), Want::SystemAndDisplay);
        assert_eq!(want_for(true, false, false, 1), Want::System);
        assert_eq!(want_for(true, false, true, 0), Want::Nothing);
        assert_eq!(want_for(false, false, true, 2), Want::Nothing, "turned off");
    }

    #[test]
    fn holding_follows_the_need() {
        let state = Mutex::new(None);
        let held =
            |state: &Mutex<Option<Held>>| state.lock().unwrap().as_ref().map(|held| held.want);
        update_in(&state, true, false, false, 2);
        assert_eq!(held(&state), Some(Want::System));
        update_in(&state, true, true, false, 2);
        assert_eq!(held(&state), Some(Want::SystemAndDisplay));
        update_in(&state, true, true, false, 0);
        assert_eq!(held(&state), None);
    }
}
