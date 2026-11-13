//! Whether the person is away from this Mac: the switch in Settings, the
//! screen locked, no input for `minutes`, or „ich gehe jetzt …“ said, until
//! they're back at the keyboard.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What tells whether the person is away.
#[derive(Clone, Copy, Debug, Default)]
pub struct Signals {
    /// The "I'm away" switch.
    pub manual: bool,
    pub locked: bool,
    /// Seconds since the last keyboard, mouse or trackpad input.
    pub idle_secs: f64,
    /// How long ago they said they were leaving, if they did.
    pub said_leaving: Option<Duration>,
}

/// Input this recent, a while after saying they'd leave, means they're back.
const BACK_WITHIN_SECS: f64 = 10.0;
/// They may still be typing right after saying it.
const LEAVING_GRACE: Duration = Duration::from_secs(120);

/// Whether the person is away, from `signals` and the idle limit in minutes
/// (0: only by switch, lock, or saying so).
pub fn is_away(minutes: u32, signals: &Signals) -> bool {
    if signals.manual || signals.locked {
        return true;
    }
    if minutes > 0 && signals.idle_secs >= f64::from(minutes) * 60.0 {
        return true;
    }
    match signals.said_leaving {
        // Said so: away until input comes back after a grace period.
        Some(ago) => ago < LEAVING_GRACE || signals.idle_secs >= BACK_WITHIN_SECS,
        None => false,
    }
}

/// Whether `text` says the person is leaving now.
pub fn says_leaving(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "ich gehe jetzt",
        "ich geh jetzt",
        "bin jetzt weg",
        "ich bin weg",
        "ich bin unterwegs",
        "bin mal weg",
        "ich gehe zum",
        "ich gehe zur",
        "i'm leaving",
        "i am leaving",
        "i'm heading out",
        "i'm out",
        "i'll be away",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

static MANUAL: AtomicBool = AtomicBool::new(false);
static LEFT: Mutex<Option<Instant>> = Mutex::new(None);

/// The "I'm away" switch.
pub fn set_manual(on: bool) {
    MANUAL.store(on, Ordering::Release);
}

pub fn manual() -> bool {
    MANUAL.load(Ordering::Acquire)
}

/// The person just said they're leaving.
pub fn left_now() {
    *LEFT.lock().unwrap_or_else(|error| error.into_inner()) = Some(Instant::now());
}

/// [`is_away`] on this Mac now; clears „ich gehe jetzt“ once they're back.
pub fn is_away_now(minutes: u32) -> bool {
    let mut left = LEFT.lock().unwrap_or_else(|error| error.into_inner());
    let signals = Signals {
        manual: manual(),
        locked: probe::screen_locked(),
        idle_secs: probe::idle_secs(),
        said_leaving: left.map(|at| at.elapsed()),
    };
    let away = is_away(minutes, &signals);
    if !away && left.is_some() {
        *left = None;
    }
    away
}

#[cfg(target_os = "macos")]
mod probe {
    use std::ffi::{CString, c_void};

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
        fn CGSessionCopyCurrentDictionary() -> *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            text: *const std::ffi::c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFBooleanGetValue(boolean: *const c_void) -> u8;
        fn CFRelease(object: *const c_void);
    }

    /// kCGEventSourceStateHIDSystemState, kCGAnyInputEventType.
    pub fn idle_secs() -> f64 {
        // SAFETY: a pure query with constant arguments.
        unsafe { CGEventSourceSecondsSinceLastEventType(1, u32::MAX) }
    }

    /// The login session's `CGSSessionScreenIsLocked`.
    pub fn screen_locked() -> bool {
        // SAFETY: the dictionary and key are created here and released
        // below; the value is borrowed from the dictionary while it lives.
        unsafe {
            let dict = CGSessionCopyCurrentDictionary();
            if dict.is_null() {
                return false;
            }
            let name = CString::new("CGSSessionScreenIsLocked").unwrap_or_default();
            // kCFStringEncodingUTF8
            let key = CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), 0x0800_0100);
            let value = if key.is_null() {
                std::ptr::null()
            } else {
                CFDictionaryGetValue(dict, key)
            };
            let locked = !value.is_null() && CFBooleanGetValue(value) != 0;
            if !key.is_null() {
                CFRelease(key);
            }
            CFRelease(dict);
            locked
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod probe {
    pub fn idle_secs() -> f64 {
        0.0
    }

    pub fn screen_locked() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> Signals {
        Signals {
            idle_secs: 2.0,
            ..Signals::default()
        }
    }

    #[test]
    fn away_by_switch_lock_or_idle() {
        assert!(!is_away(10, &here()));
        assert!(is_away(
            10,
            &Signals {
                manual: true,
                ..here()
            }
        ));
        assert!(is_away(
            10,
            &Signals {
                locked: true,
                ..here()
            }
        ));
        assert!(is_away(
            10,
            &Signals {
                idle_secs: 601.0,
                ..here()
            }
        ));
        assert!(!is_away(
            10,
            &Signals {
                idle_secs: 599.0,
                ..here()
            }
        ));
        // 0 minutes: idle alone never counts.
        assert!(!is_away(
            0,
            &Signals {
                idle_secs: 99_999.0,
                ..here()
            }
        ));
    }

    #[test]
    fn saying_so_counts_until_they_are_back() {
        let said = |ago: u64, idle: f64| Signals {
            said_leaving: Some(Duration::from_secs(ago)),
            idle_secs: idle,
            ..Signals::default()
        };
        // Still typing right after saying it.
        assert!(is_away(10, &said(5, 1.0)));
        // Gone for a while.
        assert!(is_away(10, &said(600, 300.0)));
        // Back at the keyboard.
        assert!(!is_away(10, &said(600, 1.0)));
    }

    #[test]
    fn leaving_is_told_from_other_requests() {
        for text in [
            "Ich gehe jetzt zum Friseur, bau die Seite nach",
            "Bin jetzt weg, mach das fertig",
            "I'm leaving now, finish it",
        ] {
            assert!(says_leaving(text), "{text}");
        }
        for text in [
            "Wie ist das Wetter?",
            "Geh auf github.com",
            "Gehe jetzt die Liste durch",
        ] {
            assert!(!says_leaving(text), "{text}");
        }
    }
}
