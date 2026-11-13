//! Whether the desktop can be operated right now: a locked screen (the
//! login window over everything) or a sleeping display stops every desktop
//! step a job takes, and before this a job's clicks failed silently while
//! the person was away. The assistant's guard asks [`desktop_state`] before
//! each desktop step: a sleeping display it wakes (user activity), a locked
//! screen it reports as `SCREEN_LOCKED`, so the job does what needs no
//! desktop (browser, files, shell) and asks the person for the rest.
//!
//! macOS reads `CGSSessionScreenIsLocked` from the session dictionary and
//! `CGDisplayIsAsleep` for the main display; elsewhere the desktop always
//! counts as usable.

/// The desktop as a job's next step finds it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopState {
    Usable,
    /// The display sleeps; waking it makes the desktop usable again.
    DisplayAsleep,
    /// The screen is locked: nothing but the person can open it.
    Locked,
}

/// The state from what macOS says.
pub fn state_of(locked: bool, display_asleep: bool) -> DesktopState {
    if locked {
        DesktopState::Locked
    } else if display_asleep {
        DesktopState::DisplayAsleep
    } else {
        DesktopState::Usable
    }
}

/// The desktop now; a sleeping display is woken on the way.
pub fn desktop_state() -> DesktopState {
    let state = state_of(platform::screen_locked(), platform::display_asleep());
    if state == DesktopState::DisplayAsleep {
        platform::wake_display();
        if !platform::display_asleep() {
            return DesktopState::Usable;
        }
    }
    state
}

#[cfg(all(target_os = "macos", not(test)))]
mod platform {
    use objc2::rc::Retained;
    use objc2_foundation::{NSDictionary, NSNumber, NSObject, NSString};

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> *const std::ffi::c_void;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsAsleep(display: u32) -> u32;
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionDeclareUserActivity(
            name: *const std::ffi::c_void,
            user_type: u32,
            id: *mut u32,
        ) -> i32;
    }

    pub fn screen_locked() -> bool {
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: returns a +1 CFDictionary (toll-free bridged to
            // NSDictionary) or null.
            let raw = unsafe { CGSessionCopyCurrentDictionary() };
            if raw.is_null() {
                // No session dictionary: not logged in to the console, so
                // the desktop isn't usable either.
                return true;
            }
            // SAFETY: a +1 CFDictionary is an NSDictionary we now own.
            let Some(session): Option<Retained<NSDictionary<NSString, NSObject>>> =
                (unsafe { Retained::from_raw(raw.cast_mut().cast()) })
            else {
                return false;
            };
            let key = NSString::from_str("CGSSessionScreenIsLocked");
            session
                .objectForKey(&key)
                .and_then(|value| value.downcast::<NSNumber>().ok())
                .is_some_and(|value| value.boolValue())
        })
    }

    pub fn display_asleep() -> bool {
        // SAFETY: plain CoreGraphics queries.
        unsafe { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }
    }

    pub fn wake_display() {
        let name = NSString::from_str("Agent-Desk: a task works on the desktop");
        let mut id = 0u32;
        // SAFETY: NSString is toll-free bridged to CFString; `id` is a valid
        // out pointer. kIOPMUserActiveLocal = 0.
        unsafe { IOPMAssertionDeclareUserActivity((&*name as *const NSString).cast(), 0, &mut id) };
    }
}

#[cfg(any(test, not(target_os = "macos")))]
mod platform {
    pub fn screen_locked() -> bool {
        false
    }

    pub fn display_asleep() -> bool {
        false
    }

    pub fn wake_display() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_screen_outranks_a_sleeping_display() {
        assert_eq!(state_of(false, false), DesktopState::Usable);
        assert_eq!(state_of(false, true), DesktopState::DisplayAsleep);
        assert_eq!(state_of(true, true), DesktopState::Locked);
        assert_eq!(state_of(true, false), DesktopState::Locked);
    }
}
