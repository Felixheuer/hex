//! The person keeps the front: an app a Background task opens or launches
//! may bring itself forward (TextEdit opening a document does), covering
//! the window the person types in. The guard notes which app was in front
//! before the launch and, when the launched app took the front from it,
//! gives it back at once; the new window then sits behind, or on the
//! agents' display once `place_arrivals` moved it.

/// The app in front now, by pid.
#[cfg(target_os = "macos")]
pub(super) fn frontmost() -> Option<i32> {
    crate::dictation::context::frontmost_pid()
}

#[cfg(not(target_os = "macos"))]
pub(super) fn frontmost() -> Option<i32> {
    None
}

/// The app to give the front back to: the one in front `before` the
/// launch, when the `launched` app is in front `now` instead. Not when the
/// person moved on to another app meanwhile, nor when the launched app was
/// in front already.
pub(super) fn give_back_to(
    before: Option<i32>,
    now: Option<i32>,
    launched: Option<i64>,
) -> Option<i32> {
    let (before, now, launched) = (before?, now?, launched?);
    (now != before && i64::from(now) == launched).then_some(before)
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

/// After a launch: gives the front back when the launched app took it.
pub(super) fn keep_front(before: Option<i32>, launched: Option<i64>) {
    if let Some(pid) = give_back_to(before, frontmost(), launched) {
        give_back(pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_goes_back_only_when_the_launched_app_took_it() {
        // Mail was in front; TextEdit (launched, pid 77) took it.
        assert_eq!(give_back_to(Some(10), Some(77), Some(77)), Some(10));
        // The launched app opened behind: nothing to do.
        assert_eq!(give_back_to(Some(10), Some(10), Some(77)), None);
        // The person switched to Safari meanwhile: leave them there.
        assert_eq!(give_back_to(Some(10), Some(20), Some(77)), None);
        // Unknown pids: nothing.
        assert_eq!(give_back_to(None, Some(77), Some(77)), None);
        assert_eq!(give_back_to(Some(10), Some(77), None), None);
    }
}
