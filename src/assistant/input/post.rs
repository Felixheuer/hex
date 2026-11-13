//! Pid-targeted input: events posted to one app's process
//! (`CGEventPostToPid`), so the person's pointer doesn't move and their app
//! keeps the front and the keyboard.
//!
//! - Keys and text: public API only (a key event with its Unicode string, or
//!   a virtual key code with modifier flags on the event itself; separate
//!   modifier events would leak into the person's keyboard state).
//! - Clicks: the event is built as AppKit builds one for that window
//!   (`NSEvent` with its window number), stamped with the window-local spot
//!   (`CGEventSetWindowLocation`) and SkyLight's target fields (pid, window),
//!   and the app is told its window has focus without coming forward (a
//!   `SLPSPostEventRecordTo` focus record), then a primer click outside the
//!   window eats AppKit's first-click activation. All private: looked up at
//!   run time; without them clicks are reported unavailable, never sent
//!   half-way.
//! - Scrolls: a pixel scroll event at the spot, public API.

use super::geometry::{Frame, Target};

/// What a gesture is.
#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    Click {
        button: Button,
        count: u32,
    },
    Scroll {
        dx: i32,
        dy: i32,
    },
    /// Text typed as Unicode key events.
    Text(String),
    /// One key with modifiers (`cmd+shift+d`, `return`, `pagedown`).
    Key(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Button {
    Left,
    Right,
}

/// What happened, for the tool's answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delivered {
    /// The app in front before and after (pids): they must be the same.
    pub front_before: Option<i32>,
    pub front_after: Option<i32>,
    /// The private SkyLight route was used (clicks).
    pub private: bool,
}

/// The private calls this Mac has (macOS only; all false elsewhere).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    pub window_location: bool,
    pub skylight_fields: bool,
    pub focus_record: bool,
}

impl Capabilities {
    /// Whether background clicks can be delivered.
    pub fn clicks(self) -> bool {
        self.window_location && self.skylight_fields && self.focus_record
    }
}

/// The private calls found on this Mac.
pub fn capabilities() -> Capabilities {
    #[cfg(target_os = "macos")]
    {
        macos::capabilities()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Capabilities::default()
    }
}

/// Posts `gesture` at `target` (ignored for keys and text) to window
/// `number` (frame `frame`) of process `pid`.
pub fn post(
    pid: i32,
    number: u32,
    frame: &Frame,
    target: Option<Target>,
    gesture: &Gesture,
) -> Result<Delivered, String> {
    if pid <= 0 || pid == std::process::id() as i32 {
        return Err("that isn't another app's process".into());
    }
    #[cfg(target_os = "macos")]
    {
        macos::post(pid, number, frame, target, gesture)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (number, frame, target, gesture);
        Err("background input to a window works on macOS only".into())
    }
}

/// Event flags of each modifier (`CGEventFlags`).
const CMD: u64 = 0x0010_0000;
const SHIFT: u64 = 0x0002_0000;
const OPTION: u64 = 0x0008_0000;
const CONTROL: u64 = 0x0004_0000;

/// A key combo as a virtual key code (ANSI layout) and event flags.
pub fn combo(text: &str) -> Result<(u16, u64), String> {
    let mut flags = 0;
    let mut key = None;
    for part in text.split('+').map(|part| part.trim().to_lowercase()) {
        match part.as_str() {
            "cmd" | "command" | "meta" | "⌘" | "super" => flags |= CMD,
            "shift" | "⇧" => flags |= SHIFT,
            "option" | "opt" | "alt" | "⌥" => flags |= OPTION,
            "ctrl" | "control" | "⌃" => flags |= CONTROL,
            "" => return Err(format!("“{text}” isn't a key combo")),
            name => {
                if key.is_some() {
                    return Err(format!("“{text}” names more than one key"));
                }
                key = Some(keycode(name).ok_or_else(|| format!("unknown key “{name}”"))?);
            }
        }
    }
    key.map(|code| (code, flags))
        .ok_or_else(|| format!("“{text}” has no key"))
}

/// The virtual key code (ANSI) of a key by name or character.
fn keycode(name: &str) -> Option<u16> {
    let named = match name {
        "return" | "enter" | "↩" => 36,
        "tab" => 48,
        "space" => 49,
        "delete" | "backspace" | "⌫" => 51,
        "escape" | "esc" => 53,
        "forwarddelete" | "forward_delete" | "del" | "⌦" => 117,
        "home" => 115,
        "end" => 119,
        "pageup" | "page_up" => 116,
        "pagedown" | "page_down" => 121,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        _ => {
            let mut chars = name.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            return Some(match c {
                'a' => 0,
                's' => 1,
                'd' => 2,
                'f' => 3,
                'h' => 4,
                'g' => 5,
                'z' => 6,
                'x' => 7,
                'c' => 8,
                'v' => 9,
                'b' => 11,
                'q' => 12,
                'w' => 13,
                'e' => 14,
                'r' => 15,
                'y' => 16,
                't' => 17,
                '1' => 18,
                '2' => 19,
                '3' => 20,
                '4' => 21,
                '6' => 22,
                '5' => 23,
                '=' => 24,
                '9' => 25,
                '7' => 26,
                '-' => 27,
                '8' => 28,
                '0' => 29,
                ']' => 30,
                'o' => 31,
                'u' => 32,
                '[' => 33,
                'i' => 34,
                'p' => 35,
                'l' => 37,
                'j' => 38,
                '\'' => 39,
                'k' => 40,
                ';' => 41,
                '\\' => 42,
                ',' => 43,
                '/' => 44,
                'n' => 45,
                'm' => 46,
                '.' => 47,
                '`' => 50,
                _ => return None,
            });
        }
    };
    Some(named)
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use std::time::Duration;

    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSPoint;

    use super::super::geometry::{Frame, Target};
    use super::{Button, Capabilities, Delivered, Gesture};

    type CGEventRef = *mut c_void;
    type CGEventSourceRef = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Psn {
        high: u32,
        low: u32,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceCreate(state: i32) -> CGEventSourceRef;
        fn CGEventCreateKeyboardEvent(source: CGEventSourceRef, key: u16, down: bool)
        -> CGEventRef;
        fn CGEventKeyboardSetUnicodeString(event: CGEventRef, length: usize, text: *const u16);
        fn CGEventCreateScrollWheelEvent2(
            source: CGEventSourceRef,
            units: u32,
            count: u32,
            wheel1: i32,
            wheel2: i32,
            wheel3: i32,
        ) -> CGEventRef;
        fn CGEventSetFlags(event: CGEventRef, flags: u64);
        fn CGEventSetLocation(event: CGEventRef, point: CGPoint);
        fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
        fn CGEventSetDoubleValueField(event: CGEventRef, field: u32, value: f64);
        fn CGEventSetTimestamp(event: CGEventRef, timestamp: u64);
        fn CGEventPostToPid(pid: i32, event: CGEventRef);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const c_void);
        fn CFRetain(value: *const c_void) -> *const c_void;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn GetProcessForPID(pid: i32, psn: *mut Psn) -> i32;
    }

    unsafe extern "C" {
        fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
        fn clock_gettime_nsec_np(clock: u32) -> u64;
    }

    const RTLD_LAZY: i32 = 1;
    /// `RTLD_DEFAULT` on macOS.
    const DEFAULT: *mut c_void = -2_isize as *mut c_void;
    const CLOCK_UPTIME_RAW: u32 = 8;
    const PRIVATE_STATE: i32 = -1;

    // `CGEventField`s.
    const MOUSE_NUMBER: u32 = 0;
    const CLICK_STATE: u32 = 1;
    const PRESSURE: u32 = 2;
    const BUTTON_NUMBER: u32 = 3;
    const SUBTYPE: u32 = 7;
    const TARGET_PID: u32 = 40;
    const WINDOW_FIELD: u32 = 51;
    const WINDOW_UNDER_POINTER: u32 = 91;
    const WINDOW_THAT_CAN_HANDLE: u32 = 92;
    const SCROLL_POINT_DELTA_1: u32 = 96;
    const SCROLL_POINT_DELTA_2: u32 = 97;

    // `NSEventType`s (the same numbers as `CGEventType`).
    const LEFT_DOWN: usize = 1;
    const LEFT_UP: usize = 2;
    const RIGHT_DOWN: usize = 3;
    const RIGHT_UP: usize = 4;
    const MOVED: usize = 5;

    type SetWindowLocation = unsafe extern "C" fn(CGEventRef, CGPoint);
    type SetField = unsafe extern "C" fn(CGEventRef, u32, i64);
    type PostRecord = unsafe extern "C" fn(*const Psn, *const u8) -> i32;

    struct Private {
        window_location: Option<SetWindowLocation>,
        fields: Option<SetField>,
        record: Option<PostRecord>,
    }

    fn private() -> &'static Private {
        static PRIVATE: OnceLock<Private> = OnceLock::new();
        PRIVATE.get_or_init(|| {
            // SAFETY: loading a system framework; symbols looked up by name
            // and used only with the signatures others document and the
            // experiment confirmed on macOS 26.
            unsafe {
                dlopen(
                    c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(),
                    RTLD_LAZY,
                );
                let find = |name: &std::ffi::CStr| {
                    let symbol = dlsym(DEFAULT, name.as_ptr());
                    (!symbol.is_null()).then_some(symbol)
                };
                Private {
                    window_location: find(c"CGEventSetWindowLocation").map(|symbol| {
                        std::mem::transmute::<*mut c_void, SetWindowLocation>(symbol)
                    }),
                    fields: find(c"SLEventSetIntegerValueField")
                        .map(|symbol| std::mem::transmute::<*mut c_void, SetField>(symbol)),
                    record: find(c"SLPSPostEventRecordTo")
                        .map(|symbol| std::mem::transmute::<*mut c_void, PostRecord>(symbol)),
                }
            }
        })
    }

    pub fn capabilities() -> Capabilities {
        let private = private();
        Capabilities {
            window_location: private.window_location.is_some(),
            skylight_fields: private.fields.is_some(),
            focus_record: private.record.is_some(),
        }
    }

    struct Event(CGEventRef);

    /// `-[NSEvent CGEvent]`'s return type, as the runtime encodes it.
    #[repr(transparent)]
    struct CgEventPointer(*mut c_void);

    // SAFETY: a pointer to the opaque `__CGEvent` struct, as AppKit
    // declares `CGEventRef`.
    unsafe impl objc2::encode::Encode for CgEventPointer {
        const ENCODING: objc2::encode::Encoding =
            objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("__CGEvent", &[]));
    }

    impl Drop for Event {
        fn drop(&mut self) {
            // SAFETY: owned (+1) and non-null.
            unsafe { CFRelease(self.0) };
        }
    }

    fn source() -> CGEventSourceRef {
        // A private source per process, so the person's modifier state
        // never mixes in. Kept for the process's life.
        static SOURCE: OnceLock<usize> = OnceLock::new();
        // SAFETY: creates a +1 source kept forever.
        *SOURCE.get_or_init(|| unsafe { CGEventSourceCreate(PRIVATE_STATE) } as usize)
            as CGEventSourceRef
    }

    fn frontmost() -> Option<i32> {
        crate::dictation::context::frontmost_pid()
    }

    fn now() -> u64 {
        // SAFETY: a plain clock read.
        unsafe { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }
    }

    pub fn post(
        pid: i32,
        number: u32,
        frame: &Frame,
        target: Option<Target>,
        gesture: &Gesture,
    ) -> Result<Delivered, String> {
        let front_before = frontmost();
        let private = match gesture {
            Gesture::Click { button, count } => {
                let target = target.ok_or("a click needs a spot in the window")?;
                click(pid, number, frame, target, *button, (*count).clamp(1, 3))?;
                true
            }
            Gesture::Scroll { dx, dy } => {
                let target = target.ok_or("a scroll needs a spot in the window")?;
                scroll(pid, number, target, *dx, *dy)?;
                false
            }
            Gesture::Text(text) => {
                type_text(pid, text);
                false
            }
            Gesture::Key(combo) => {
                let (code, flags) = super::combo(combo)?;
                key(pid, code, flags);
                false
            }
        };
        std::thread::sleep(Duration::from_millis(60));
        Ok(Delivered {
            front_before,
            front_after: frontmost(),
            private,
        })
    }

    /// Opens this process's window server connection, once.
    fn connect() {
        static CONNECTED: OnceLock<()> = OnceLock::new();
        CONNECTED.get_or_init(|| {
            type Main = unsafe extern "C" fn() -> i32;
            // SAFETY: a lookup by name; the call takes nothing.
            unsafe {
                let symbol = dlsym(DEFAULT, c"CGSMainConnectionID".as_ptr());
                if !symbol.is_null() {
                    let main: Main = std::mem::transmute::<*mut c_void, Main>(symbol);
                    main();
                }
            }
        });
    }

    /// Tells app `pid` its window `number` has focus, without bringing it
    /// forward or activating the app (a SkyLight focus event record).
    fn focus_record(pid: i32, number: u32) -> Result<(), String> {
        let record = private().record.ok_or("SLPSPostEventRecordTo is missing")?;
        let mut psn = Psn::default();
        // A process without a window server connection yet (the guard is
        // a command-line process) gets one first: the Process Manager
        // calls answer only through it.
        connect();
        // SAFETY: `psn` is written on success.
        let status = unsafe { GetProcessForPID(pid, &raw mut psn) };
        if status != 0 {
            return Err(format!("the app's process wasn't found ({status})"));
        }
        let mut bytes = [0_u8; 0xf8];
        bytes[0x04] = 0xf8;
        bytes[0x08] = 0x0d;
        bytes[0x3c..0x40].copy_from_slice(&number.to_le_bytes());
        bytes[0x8a] = 0x01;
        // SAFETY: a 0xf8-byte record, as the window server expects.
        unsafe { record(&raw const psn, bytes.as_ptr()) };
        Ok(())
    }

    /// A mouse event as AppKit would make it for window `number`, stamped
    /// with where it lands.
    fn mouse(
        kind: usize,
        number: u32,
        pid: i32,
        target: Target,
        clicks: u32,
        event_number: i64,
        button: i64,
    ) -> Result<Event, String> {
        let private = private();
        let (Some(set_location), Some(set_field)) = (private.window_location, private.fields)
        else {
            return Err("this Mac lacks the calls background clicks need".into());
        };
        let class = AnyClass::get(c"NSEvent").ok_or("no NSEvent")?;
        // SAFETY: NSEvent's documented factory; the CGEvent it gives is
        // retained before the autorelease pool drains.
        let event = objc2::rc::autoreleasepool(|_| unsafe {
            let ns: *mut AnyObject = msg_send![class,
                mouseEventWithType: kind,
                location: NSPoint::new(0.0, 0.0),
                modifierFlags: 0_usize,
                timestamp: 0.0_f64,
                windowNumber: number as isize,
                context: std::ptr::null_mut::<AnyObject>(),
                eventNumber: 0_isize,
                clickCount: clicks as isize,
                pressure: 1.0_f32];
            if ns.is_null() {
                return std::ptr::null_mut();
            }
            let cg: CgEventPointer = msg_send![ns, CGEvent];
            if cg.0.is_null() {
                std::ptr::null_mut()
            } else {
                CFRetain(cg.0) as *mut c_void
            }
        });
        if event.is_null() {
            return Err("the mouse event couldn't be made".into());
        }
        let event = Event(event);
        let window = i64::from(number);
        // SAFETY: a live event; fields and setters as AppKit uses them.
        unsafe {
            CGEventSetIntegerValueField(event.0, BUTTON_NUMBER, button);
            CGEventSetIntegerValueField(event.0, SUBTYPE, 3);
            CGEventSetIntegerValueField(event.0, CLICK_STATE, i64::from(clicks));
            CGEventSetDoubleValueField(event.0, PRESSURE, if clicks > 0 { 1.0 } else { 0.0 });
            CGEventSetLocation(
                event.0,
                CGPoint {
                    x: target.screen.0,
                    y: target.screen.1,
                },
            );
            CGEventSetIntegerValueField(event.0, WINDOW_UNDER_POINTER, window);
            CGEventSetIntegerValueField(event.0, WINDOW_THAT_CAN_HANDLE, window);
            set_location(
                event.0,
                CGPoint {
                    x: target.local.0,
                    y: target.local.1,
                },
            );
            set_field(event.0, MOUSE_NUMBER, event_number);
            set_field(event.0, TARGET_PID, i64::from(pid));
            set_field(event.0, WINDOW_FIELD, window);
            set_field(event.0, WINDOW_UNDER_POINTER, window);
            set_field(event.0, WINDOW_THAT_CAN_HANDLE, window);
            CGEventSetTimestamp(event.0, now());
        }
        Ok(event)
    }

    fn send(pid: i32, event: &Event) {
        // SAFETY: a live event, to one process.
        unsafe { CGEventPostToPid(pid, event.0) };
    }

    /// One click (or a double/triple one) at `target`.
    fn press(
        pid: i32,
        number: u32,
        target: Target,
        button: Button,
        count: u32,
    ) -> Result<(), String> {
        let (down, up, button) = match button {
            Button::Left => (LEFT_DOWN, LEFT_UP, 0),
            Button::Right => (RIGHT_DOWN, RIGHT_UP, 1),
        };
        send(pid, &mouse(MOVED, number, pid, target, 0, 2, button)?);
        std::thread::sleep(Duration::from_millis(15));
        for clicks in 1..=count {
            send(pid, &mouse(down, number, pid, target, clicks, 3, button)?);
            std::thread::sleep(Duration::from_millis(1));
            send(pid, &mouse(up, number, pid, target, clicks, 3, button)?);
            std::thread::sleep(Duration::from_millis(40));
        }
        Ok(())
    }

    fn click(
        pid: i32,
        number: u32,
        frame: &Frame,
        target: Target,
        button: Button,
        count: u32,
    ) -> Result<(), String> {
        if !capabilities().clicks() {
            return Err("this Mac lacks the calls background clicks need".into());
        }
        focus_record(pid, number)?;
        std::thread::sleep(Duration::from_millis(30));
        // AppKit takes a background window's first click to activate it;
        // this one lands outside the window, so nothing in it is hit.
        press(pid, number, frame.primer(), Button::Left, 1)?;
        std::thread::sleep(Duration::from_millis(60));
        press(pid, number, target, button, count)
    }

    fn scroll(pid: i32, number: u32, target: Target, dx: i32, dy: i32) -> Result<(), String> {
        // SAFETY: a +1 event (pixel units), owned by `Event`.
        let event = unsafe { CGEventCreateScrollWheelEvent2(source(), 0, 2, dy, dx, 0) };
        if event.is_null() {
            return Err("the scroll event couldn't be made".into());
        }
        let event = Event(event);
        // SAFETY: a live event.
        unsafe {
            CGEventSetLocation(
                event.0,
                CGPoint {
                    x: target.screen.0,
                    y: target.screen.1,
                },
            );
            CGEventSetIntegerValueField(event.0, SCROLL_POINT_DELTA_1, i64::from(dy));
            CGEventSetIntegerValueField(event.0, SCROLL_POINT_DELTA_2, i64::from(dx));
            CGEventSetIntegerValueField(event.0, WINDOW_UNDER_POINTER, i64::from(number));
            CGEventSetIntegerValueField(event.0, WINDOW_THAT_CAN_HANDLE, i64::from(number));
            if let (Some(set_location), Some(set_field)) =
                (private().window_location, private().fields)
            {
                set_location(
                    event.0,
                    CGPoint {
                        x: target.local.0,
                        y: target.local.1,
                    },
                );
                set_field(event.0, TARGET_PID, i64::from(pid));
                set_field(event.0, WINDOW_FIELD, i64::from(number));
            }
        }
        send(pid, &event);
        Ok(())
    }

    fn key_event(code: u16, down: bool, flags: u64, text: Option<u16>) -> Option<Event> {
        // SAFETY: a +1 event owned by `Event`; the string is one UTF-16 unit.
        unsafe {
            let event = CGEventCreateKeyboardEvent(source(), code, down);
            if event.is_null() {
                return None;
            }
            CGEventSetFlags(event, flags);
            if let Some(unit) = text {
                CGEventKeyboardSetUnicodeString(event, 1, &raw const unit);
            }
            Some(Event(event))
        }
    }

    fn type_text(pid: i32, text: &str) {
        for unit in text.encode_utf16() {
            // Return and Tab as their keys, so fields that act on them do.
            let (code, unicode) = match unit {
                0x0a | 0x0d => (36, None),
                0x09 => (48, None),
                _ => (0, Some(unit)),
            };
            for down in [true, false] {
                if let Some(event) = key_event(code, down, 0, unicode) {
                    send(pid, &event);
                }
            }
            std::thread::sleep(Duration::from_millis(6));
        }
    }

    fn key(pid: i32, code: u16, flags: u64) {
        for down in [true, false] {
            if let Some(event) = key_event(code, down, flags, None) {
                send(pid, &event);
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_become_a_key_and_flags_on_the_event() {
        assert_eq!(combo("return"), Ok((36, 0)));
        assert_eq!(combo("Cmd+Shift+D"), Ok((2, CMD | SHIFT)));
        assert_eq!(combo("ctrl+alt+pagedown"), Ok((121, CONTROL | OPTION)));
        assert_eq!(combo("cmd+a"), Ok((0, CMD)));
        assert!(combo("cmd+").is_err());
        assert!(combo("cmd").is_err());
        assert!(combo("a+b").is_err());
        assert!(combo("hyper+x").is_err());
    }

    #[test]
    fn clicks_need_every_private_call() {
        let all = Capabilities {
            window_location: true,
            skylight_fields: true,
            focus_record: true,
        };
        assert!(all.clicks());
        let missing = Capabilities {
            focus_record: false,
            ..all
        };
        assert!(!missing.clicks());
    }
}

/// Live, by hand only (`cargo test --bin hex live_background_input --
/// --ignored`, with `HEX_LIVE_FILE` naming a throwaway text file): opens it
/// in TextEdit in the background, types and clicks into it by pid while
/// another app stays in front, reads it back through Accessibility, and
/// quits TextEdit. Never run against the person's own documents.
#[cfg(all(test, target_os = "macos"))]
mod live {
    use super::super::geometry::Frame;
    use super::*;

    fn front() -> Option<i32> {
        crate::dictation::context::frontmost_pid()
    }

    fn textedit_value() -> String {
        let output = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "tell application \"System Events\" to get value of text area 1 of scroll area 1 of window 1 of process \"TextEdit\""])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[test]
    #[ignore = "drives TextEdit on this Mac; run by hand"]
    fn live_background_input() {
        let file = std::env::var("HEX_LIVE_FILE").expect("HEX_LIVE_FILE");
        assert!(
            std::process::Command::new("/usr/bin/pgrep")
                .args(["-x", "TextEdit"])
                .output()
                .unwrap()
                .stdout
                .is_empty(),
            "TextEdit is open already: it may hold the person's documents"
        );
        let before = front();
        // TextEdit quits however the test ends.
        struct Quit;
        impl Drop for Quit {
            fn drop(&mut self) {
                let _ = std::process::Command::new("/usr/bin/osascript")
                    .args(["-e", "tell application id \"com.apple.TextEdit\" to close every document saving no", "-e", "tell application id \"com.apple.TextEdit\" to quit saving no"])
                    .status();
            }
        }
        let _quit = Quit;
        std::process::Command::new("/usr/bin/open")
            .args(["-g", "-a", "TextEdit", &file])
            .status()
            .unwrap();
        let mut window = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let pid: Option<i32> = String::from_utf8_lossy(
                &std::process::Command::new("/usr/bin/pgrep")
                    .args(["-x", "TextEdit"])
                    .output()
                    .unwrap()
                    .stdout,
            )
            .split_whitespace()
            .next()
            .and_then(|pid| pid.parse().ok());
            window = pid.and_then(|pid| {
                crate::assistant::computer::now::all_windows()
                    .into_iter()
                    .find(|window| {
                        window.pid == pid
                            && window.layer == 0
                            && window.bounds[2] > 100.0
                            && crate::assistant::actions::ax::windows(pid)
                                .iter()
                                .any(|listed| listed.number == window.number)
                    })
            });
            if window.is_some() {
                break;
            }
        }
        let window = window.expect("TextEdit's window");
        let [x, y, width, height] = window.bounds;
        let frame = Frame {
            x,
            y,
            width,
            height,
            scale: 2.0,
        };
        println!("capabilities: {:?}", capabilities());
        // Off the screen, as a task's window waits.
        let displays = crate::assistant::computer::displays();
        let main = displays.iter().find(|display| display.is_primary).unwrap();
        let parked = (
            main.frame.0 + main.frame.2 - 2.0,
            main.frame.1 + main.frame.3 - 2.0,
        );
        let mut moved = Err(0);
        for _ in 0..20 {
            moved = crate::assistant::actions::ax::set_position(window.pid, window.number, parked);
            if moved.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        moved.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let frame = Frame {
            x: parked.0,
            y: parked.1,
            ..frame
        };
        let typed = post(
            window.pid,
            window.number,
            &frame,
            None,
            &Gesture::Key("cmd+down".into()),
        )
        .unwrap();
        let typed2 = post(
            window.pid,
            window.number,
            &frame,
            None,
            &Gesture::Text(" [pid keys]".into()),
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let after_keys = textedit_value();
        println!(
            "after keys: {after_keys:?} front kept: {} {}",
            typed.front_before == typed.front_after,
            typed2.front_before == typed2.front_after
        );
        // Click at the very start of the text (about 10,40 points into the
        // window: below its title bar), then type there.
        let target = frame
            .target((8.0, 36.0), super::super::geometry::Units::Points)
            .unwrap();
        let clicked = post(
            window.pid,
            window.number,
            &frame,
            Some(target),
            &Gesture::Click {
                button: Button::Left,
                count: 1,
            },
        )
        .unwrap();
        post(
            window.pid,
            window.number,
            &frame,
            None,
            &Gesture::Text("^".into()),
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let after_click = textedit_value();
        println!(
            "after click: {after_click:?} private route: {}",
            clicked.private
        );
        let after = front();
        let _ = std::process::Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "tell application id \"com.apple.TextEdit\" to close every document saving no",
                "-e",
                "tell application id \"com.apple.TextEdit\" to quit saving no",
            ])
            .status();
        assert_eq!(before, after, "the person's app kept the front");
        assert!(after_keys.ends_with("[pid keys]"), "{after_keys}");
        assert!(after_click.starts_with('^'), "{after_click}");
    }
}
