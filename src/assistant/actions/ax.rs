//! What Accessibility and the workspace say about an app's windows, and the
//! two app-level steps recovery takes: show a hidden app, quit one.
//!
//! Accessibility lists an app's windows on the desktop the person is on
//! (and its minimized ones), never those on another desktop: a window the
//! window server knows but Accessibility doesn't list is on another desktop.

/// One window Accessibility lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AxWindow {
    /// Its CoreGraphics window number (agent-desktop's `w-<number>`).
    pub number: u32,
    pub minimized: bool,
    /// In full screen: a desktop (Space) of its own.
    pub full_screen: bool,
    /// It says its document has unsaved changes.
    pub edited: bool,
}

/// The windows Accessibility lists for process `pid` (empty when it can't
/// be asked).
pub fn windows(pid: i32) -> Vec<AxWindow> {
    #[cfg(target_os = "macos")]
    {
        macos::windows(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        Vec::new()
    }
}

/// Ends full screen for window `number` of app `pid` (`AXFullScreen`
/// off); whether Accessibility took it.
pub fn leave_full_screen(pid: i32, number: u32) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::leave_full_screen(pid, number)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, number);
        false
    }
}

/// Whether the app (`pid`) is hidden (⌘H).
pub fn hidden(pid: i32) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::with_app(pid, |app| app.isHidden()).unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        false
    }
}

/// Shows the hidden app (`pid`) without activating it.
pub fn unhide(pid: i32) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::with_app(pid, |app| app.unhide()).unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        false
    }
}

/// Asks the app (`pid`) to quit as ⌘Q does (it may still ask to save).
pub fn terminate(pid: i32) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::with_app(pid, |app| app.terminate()).unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        false
    }
}

/// Minimizes (`true`) or restores (`false`) window `number` of app `pid`
/// through Accessibility; the Accessibility error code when it can't
/// (`-25204` cannot complete, `-25205` not implemented, …).
pub fn set_minimized(pid: i32, number: u32, minimized: bool) -> Result<(), i32> {
    #[cfg(target_os = "macos")]
    {
        macos::set_flag(pid, number, c"AXMinimized", minimized)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, number, minimized);
        Err(-1)
    }
}

/// Moves window `number` of app `pid` so its top-left corner is at
/// `(x, y)` (global points); works on a minimized window too, which then
/// comes back there. The Accessibility error code when it can't.
pub fn set_position(pid: i32, number: u32, (x, y): (f64, f64)) -> Result<(), i32> {
    #[cfg(target_os = "macos")]
    {
        macos::set_position(pid, number, x, y)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, number, x, y);
        Err(-1)
    }
}

/// Asks the app's own accessibility to come on (`AXManualAccessibility`,
/// which apps built on Chromium and some others wait for before they
/// answer); harmless for apps that don't know it.
pub fn wake(pid: i32) {
    #[cfg(target_os = "macos")]
    macos::wake(pid);
    #[cfg(not(target_os = "macos"))]
    let _ = pid;
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, c_void};
    use std::ptr;

    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSRunningApplication;

    use super::AxWindow;

    type CfTypeRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> CfTypeRef;
        fn AXUIElementCopyAttributeValue(
            element: CfTypeRef,
            attribute: CfTypeRef,
            value: *mut CfTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: CfTypeRef, seconds: f32) -> i32;
        fn _AXUIElementGetWindow(element: CfTypeRef, window: *mut u32) -> i32;
        fn AXUIElementSetAttributeValue(
            element: CfTypeRef,
            attribute: CfTypeRef,
            value: CfTypeRef,
        ) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: CfTypeRef);
        fn CFRetain(value: CfTypeRef) -> CfTypeRef;
        fn CFGetTypeID(value: CfTypeRef) -> usize;
        fn CFArrayGetTypeID() -> usize;
        fn CFArrayGetCount(array: CfTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CfTypeRef, index: isize) -> CfTypeRef;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(value: CfTypeRef) -> u8;
        static kCFBooleanFalse: CfTypeRef;
        static kCFBooleanTrue: CfTypeRef;
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            string: *const std::ffi::c_char,
            encoding: u32,
        ) -> CfTypeRef;
    }

    const UTF8: u32 = 0x0800_0100;
    /// An unresponsive app (one on another desktop answers slowly) never
    /// holds the guard long.
    const TIMEOUT: f32 = 0.25;

    struct Cf(CfTypeRef);

    impl Drop for Cf {
        fn drop(&mut self) {
            // SAFETY: owned (+1) and non-null.
            unsafe { CFRelease(self.0) };
        }
    }

    fn name(text: &CStr) -> Option<Cf> {
        // SAFETY: a NUL-terminated C string; +1 result or null.
        let string = unsafe { CFStringCreateWithCString(ptr::null(), text.as_ptr(), UTF8) };
        (!string.is_null()).then_some(Cf(string))
    }

    fn attribute(element: &Cf, key: &CStr) -> Option<Cf> {
        let key = name(key)?;
        let mut value = ptr::null();
        // SAFETY: live element and key; `value` is +1 on success.
        let status = unsafe { AXUIElementCopyAttributeValue(element.0, key.0, &mut value) };
        (status == 0 && !value.is_null()).then(|| Cf(value))
    }

    fn flag(element: &Cf, key: &CStr) -> bool {
        attribute(element, key).is_some_and(|value| {
            // SAFETY: a live CF object; read only when it is a CFBoolean.
            unsafe {
                CFGetTypeID(value.0) == CFBooleanGetTypeID() && CFBooleanGetValue(value.0) != 0
            }
        })
    }

    /// Each window element of app `pid` with its window number.
    fn elements(pid: i32) -> Vec<(u32, Cf)> {
        if pid <= 0 || pid == std::process::id() as i32 {
            return Vec::new();
        }
        // SAFETY: +1 application element or null.
        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            return Vec::new();
        }
        let app = Cf(app);
        // SAFETY: a live element.
        unsafe { AXUIElementSetMessagingTimeout(app.0, TIMEOUT) };
        let Some(list) = attribute(&app, c"AXWindows") else {
            return Vec::new();
        };
        // SAFETY: type checked before use as an array.
        if unsafe { CFGetTypeID(list.0) != CFArrayGetTypeID() } {
            return Vec::new();
        }
        let count = unsafe { CFArrayGetCount(list.0) };
        (0..count)
            .filter_map(|index| {
                // SAFETY: Get rule; retained before the array goes.
                let item = unsafe { CFArrayGetValueAtIndex(list.0, index) };
                if item.is_null() {
                    return None;
                }
                let window = Cf(unsafe { CFRetain(item) });
                unsafe { AXUIElementSetMessagingTimeout(window.0, TIMEOUT) };
                let mut number = 0_u32;
                // SAFETY: a live window element; `number` written on success.
                let status = unsafe { _AXUIElementGetWindow(window.0, &raw mut number) };
                (status == 0 && number != 0).then_some((number, window))
            })
            .collect()
    }

    pub fn windows(pid: i32) -> Vec<AxWindow> {
        elements(pid)
            .into_iter()
            .map(|(number, window)| AxWindow {
                number,
                minimized: flag(&window, c"AXMinimized"),
                full_screen: flag(&window, c"AXFullScreen"),
                // Document windows say so (`AXIsEdited`; some apps
                // `AXModified`).
                edited: flag(&window, c"AXIsEdited") || flag(&window, c"AXModified"),
            })
            .collect()
    }

    pub fn leave_full_screen(pid: i32, number: u32) -> bool {
        let Some((_, window)) = elements(pid).into_iter().find(|(n, _)| *n == number) else {
            return false;
        };
        let Some(key) = name(c"AXFullScreen") else {
            return false;
        };
        // SAFETY: a live window element, a CFString key, the shared false.
        unsafe { AXUIElementSetAttributeValue(window.0, key.0, kCFBooleanFalse) == 0 }
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXValueCreate(kind: u32, value: *const c_void) -> CfTypeRef;
    }

    /// `kAXValueCGPointType`.
    const POINT: u32 = 1;

    fn window(pid: i32, number: u32) -> Result<Cf, i32> {
        elements(pid)
            .into_iter()
            .find(|(n, _)| *n == number)
            .map(|(_, window)| window)
            .ok_or(-25202)
    }

    pub fn set_flag(pid: i32, number: u32, key: &CStr, on: bool) -> Result<(), i32> {
        let window = window(pid, number)?;
        let key = name(key).ok_or(-1)?;
        // SAFETY: a live window element, a CFString key, a shared boolean.
        let status = unsafe {
            AXUIElementSetAttributeValue(
                window.0,
                key.0,
                if on { kCFBooleanTrue } else { kCFBooleanFalse },
            )
        };
        if status == 0 { Ok(()) } else { Err(status) }
    }

    pub fn set_position(pid: i32, number: u32, x: f64, y: f64) -> Result<(), i32> {
        let window = window(pid, number)?;
        let key = name(c"AXPosition").ok_or(-1)?;
        let point = [x, y];
        // SAFETY: a CGPoint (two doubles) wrapped in a +1 AXValue.
        let value = unsafe { AXValueCreate(POINT, point.as_ptr().cast()) };
        if value.is_null() {
            return Err(-1);
        }
        let value = Cf(value);
        // SAFETY: live element, key, and value.
        let status = unsafe { AXUIElementSetAttributeValue(window.0, key.0, value.0) };
        if status == 0 { Ok(()) } else { Err(status) }
    }

    pub fn wake(pid: i32) {
        if pid <= 0 || pid == std::process::id() as i32 {
            return;
        }
        // SAFETY: +1 application element or null.
        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            return;
        }
        let app = Cf(app);
        if let Some(key) = name(c"AXManualAccessibility") {
            // SAFETY: live element and key; the shared true.
            unsafe {
                AXUIElementSetMessagingTimeout(app.0, TIMEOUT);
                AXUIElementSetAttributeValue(app.0, key.0, kCFBooleanTrue);
            }
        }
    }

    pub fn with_app<T>(pid: i32, body: impl FnOnce(&NSRunningApplication) -> T) -> Option<T> {
        autoreleasepool(|_| {
            let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
            Some(body(&app))
        })
    }
}
