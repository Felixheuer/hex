//! The displays as the OS knows them, from any thread: each one's id, a
//! key that stays with it, its bounds in global points, and its scale; and
//! a callback when displays come, go, or change, so no panel is left on a
//! display that's gone. macOS asks CoreGraphics (the key is the display's
//! UUID); Linux asks X11's RandR (XWayland's too, where Rune puts overlays
//! on GNOME), Windows its monitors, and both notice changes by reading
//! again (the watcher reads once a second while there's work).

use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;

#[cfg(not(target_os = "macos"))]
use super::super::screens::Screen;

#[cfg(not(target_os = "macos"))]
pub(super) use others::{front_window, pointer, read, take_changed, watch};

#[cfg(not(target_os = "macos"))]
mod others {
    use std::sync::Mutex;

    use super::Screen;

    /// The displays as last read, so a change is noticed.
    static LAST: Mutex<Vec<Screen>> = Mutex::new(Vec::new());

    /// Every display, the primary first, in global points.
    pub(in super::super) fn read() -> Vec<Screen> {
        let mut screens = native();
        // A Wayland desktop without XWayland tells Agent-Desk nothing of its
        // displays: the overlays go where a common one would have them.
        if screens.is_empty() {
            let frame = [0.0, 0.0, 1920.0, 1080.0];
            screens.push(Screen {
                id: 1,
                key: key(1),
                frame,
                visible: frame,
                scale: 1.0,
            });
        }
        let mut last = LAST.lock().unwrap_or_else(|error| error.into_inner());
        if *last != screens {
            if !last.is_empty() {
                super::CHANGED.store(true, std::sync::atomic::Ordering::Release);
            }
            *last = screens.clone();
        }
        screens
    }

    #[cfg(target_os = "windows")]
    fn native() -> Vec<Screen> {
        // Win32 reports physical pixels; Rune places overlays in points.
        scrawl_capture::win::monitors()
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                let scale = monitor.scale.max(0.5);
                let points = |[x, y, w, h]: [f64; 4]| [x / scale, y / scale, w / scale, h / scale];
                let id = u32::try_from(index + 1).unwrap_or(u32::MAX);
                Screen {
                    id,
                    key: key(id),
                    frame: points(monitor.rect),
                    visible: points(monitor.work),
                    scale,
                }
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    fn native() -> Vec<Screen> {
        crate::assistant::computer::x11_monitors()
            .into_iter()
            .map(|display| {
                let id = display.id.parse().unwrap_or(1);
                let (x, y, w, h) = display.frame;
                Screen {
                    id,
                    key: format!("{}-{w}x{h}", display.name),
                    frame: [x, y, w, h],
                    visible: [x, y, w, h],
                    scale: display.scale,
                }
            })
            .collect()
    }

    /// The display's key: its place in the list.
    pub(in super::super) fn key(id: u32) -> String {
        format!("display-{id}")
    }

    /// Nothing to register: changes are noticed by reading again.
    pub(in super::super) fn watch(_wake: fn()) {}

    /// Whether displays changed since the last look.
    pub(in super::super) fn take_changed() -> bool {
        let _ = read();
        super::CHANGED.swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// The pointer, in global points.
    pub(in super::super) fn pointer() -> Option<(f64, f64)> {
        #[cfg(target_os = "windows")]
        {
            let (x, y) = scrawl_capture::win::cursor_position()?;
            let scale = scrawl_capture::win::monitor_at(x, y).map_or(1.0, |m| m.scale.max(0.5));
            Some((x / scale, y / scale))
        }
        #[cfg(target_os = "linux")]
        {
            crate::assistant::computer::x11_pointer()
        }
    }

    /// The window in front, in global points.
    pub(in super::super) fn front_window() -> Option<[f64; 4]> {
        #[cfg(target_os = "windows")]
        {
            let window = scrawl_capture::win::foreground_window()?;
            let [x, y, w, h] = window.rect;
            let scale = scrawl_capture::win::monitor_at(x + w / 2.0, y + h / 2.0)
                .map_or(1.0, |m| m.scale.max(0.5));
            Some([x / scale, y / scale, w / scale, h / scale])
        }
        #[cfg(target_os = "linux")]
        {
            crate::assistant::computer::x11_front_window()
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) use mac::{key, read, watch};

/// Set when displays changed since [`take_changed`] last looked.
static CHANGED: AtomicBool = AtomicBool::new(false);
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
static WATCHING: OnceLock<()> = OnceLock::new();

/// Whether displays changed since the last look.
#[cfg(target_os = "macos")]
pub(super) fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, OnceLock};

    use super::super::super::screens::Screen;
    use super::{CHANGED, WATCHING};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CgRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UuidBytes([u8; 16]);

    type Reconfigured = unsafe extern "C" fn(display: u32, flags: u32, user: *mut std::ffi::c_void);

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> CgRect;
        fn CGDisplayCopyDisplayMode(display: u32) -> *mut std::ffi::c_void;
        fn CGDisplayModeGetWidth(mode: *mut std::ffi::c_void) -> usize;
        fn CGDisplayModeGetPixelWidth(mode: *mut std::ffi::c_void) -> usize;
        fn CGDisplayModeRelease(mode: *mut std::ffi::c_void);
        fn CGDisplayRegisterReconfigurationCallback(
            callback: Reconfigured,
            user: *mut std::ffi::c_void,
        ) -> i32;
    }

    #[link(name = "ColorSync", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *const std::ffi::c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFUUIDGetUUIDBytes(uuid: *const std::ffi::c_void) -> UuidBytes;
        fn CFRelease(object: *const std::ffi::c_void);
    }

    /// The active displays, the main one first, in global points (y down from
    /// the main display's top: Rune's global space and Accessibility's). Their
    /// visible area is their whole frame here; the main thread reads the menu
    /// bar and Dock (`host::pointer_display`).
    pub(in super::super) fn read() -> Vec<Screen> {
        let mut ids = [0u32; 32];
        let mut count = 0u32;
        // SAFETY: the buffer holds `ids.len()` entries and CoreGraphics writes
        // at most that many; these calls are thread-safe.
        let status =
            unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
        if status != 0 {
            return Vec::new();
        }
        ids[..count as usize]
            .iter()
            .map(|&id| {
                // SAFETY: a display id CoreGraphics just listed.
                let r = unsafe { CGDisplayBounds(id) };
                let frame = [r.x, r.y, r.width, r.height];
                Screen {
                    id,
                    key: key(id),
                    frame,
                    visible: frame,
                    scale: scale(id),
                }
            })
            .collect()
    }

    /// Pixels per point in the display's current mode (2 on a Retina display,
    /// whatever resolution it's set to look like).
    fn scale(id: u32) -> f64 {
        // SAFETY: the mode is released after reading it; a null mode (a display
        // just unplugged) reads as 1×.
        unsafe {
            let mode = CGDisplayCopyDisplayMode(id);
            if mode.is_null() {
                return 1.0;
            }
            let (points, pixels) = (
                CGDisplayModeGetWidth(mode),
                CGDisplayModeGetPixelWidth(mode),
            );
            CGDisplayModeRelease(mode);
            if points == 0 {
                1.0
            } else {
                pixels as f64 / points as f64
            }
        }
    }

    /// Display UUIDs by id, so each is asked for once.
    static KEYS: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

    /// The display's UUID, which stays with it across ports and restarts; its
    /// id when it has none.
    pub(in super::super) fn key(id: u32) -> String {
        let mut keys = KEYS.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((_, key)) = keys.iter().find(|(known, _)| *known == id) {
            return key.clone();
        }
        // SAFETY: the UUID comes from a Create function and is released here.
        let key = unsafe {
            let uuid = CGDisplayCreateUUIDFromDisplayID(id);
            if uuid.is_null() {
                format!("display-{id}")
            } else {
                let UuidBytes(bytes) = CFUUIDGetUUIDBytes(uuid);
                CFRelease(uuid);
                bytes.iter().map(|byte| format!("{byte:02X}")).collect()
            }
        };
        keys.push((id, key.clone()));
        key
    }

    /// Calls `wake` (from the main thread's run loop) whenever a display is
    /// added, removed, moved, or changes mode. Registered once.
    pub(in super::super) fn watch(wake: fn()) {
        static WAKE: OnceLock<fn()> = OnceLock::new();
        let _ = WAKE.set(wake);
        unsafe extern "C" fn reconfigured(_display: u32, flags: u32, _user: *mut std::ffi::c_void) {
            // `kCGDisplayBeginConfigurationFlag`: the change hasn't happened yet.
            if flags & 1 != 0 {
                return;
            }
            CHANGED.store(true, Ordering::Release);
            if let Some(wake) = WAKE.get() {
                wake();
            }
        }
        WATCHING.get_or_init(|| {
            // SAFETY: the callback is a plain function that lives for the
            // process and touches only atomics and a condvar.
            unsafe { CGDisplayRegisterReconfigurationCallback(reconfigured, std::ptr::null_mut()) };
        });
    }
}
