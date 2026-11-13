//! The person's data the assistant reads on this Mac, and macOS's answer
//! for each: Contacts (`hex_lookup` finds who to reach), Reminders, the
//! Calendar (the agenda), and Full Disk Access (Mail and Messages, read
//! from copies). The assistant's reads never ask (`assistant::knowledge`):
//! onboarding's permissions page asks here, once each, and the repair guide
//! comes back while one was never answered (`setup_gaps`). A refusal is the
//! person's answer and is not asked again; System Settings turns it on.
//!
//! ```ts
//! Contacts    CNContactStore.requestAccess            Privacy_Contacts
//! Reminders   EKEventStore.requestFullAccessToReminders  Privacy_Reminders
//! Calendar    calendar::request_access                Privacy_Calendars
//! Full Disk   no prompt: System Settings only         Privacy_AllFiles
//! ```
//!
//! The answers are read off the window's thread ([`refresh_async`]) and
//! kept here, as `automation` keeps its own, so the page reads them
//! without blocking. OWNER: onboarding.

use std::sync::atomic::{AtomicU8, Ordering};

/// A kind of the person's data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    Contacts,
    Reminders,
    Calendar,
    FullDisk,
}

impl Source {
    pub const ALL: [Source; 4] = [
        Source::Contacts,
        Source::Reminders,
        Source::Calendar,
        Source::FullDisk,
    ];

    /// The ones macOS asks about with its own prompt; Full Disk Access has
    /// none.
    pub const ASKED: [Source; 3] = [Source::Contacts, Source::Reminders, Source::Calendar];

    fn index(self) -> usize {
        match self {
            Source::Contacts => 0,
            Source::Reminders => 1,
            Source::Calendar => 2,
            Source::FullDisk => 3,
        }
    }

    /// Its System Settings › Privacy & Security pane.
    pub fn settings_pane(self) -> &'static str {
        match self {
            Source::Contacts => "Privacy_Contacts",
            Source::Reminders => "Privacy_Reminders",
            Source::Calendar => "Privacy_Calendars",
            Source::FullDisk => "Privacy_AllFiles",
        }
    }
}

/// macOS's answer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Access {
    Granted,
    /// Never asked: Allow shows macOS's prompt.
    NotAsked,
    /// Refused or restricted: only System Settings turns it on.
    Denied,
    /// Not read yet.
    #[default]
    Unknown,
}

impl Access {
    fn code(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Granted => 1,
            Self::NotAsked => 2,
            Self::Denied => 3,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Granted,
            2 => Self::NotAsked,
            3 => Self::Denied,
            _ => Self::Unknown,
        }
    }

    /// The person answered (either way), or it can't be asked.
    pub fn answered(self) -> bool {
        self != Self::NotAsked
    }
}

impl From<crate::assistant::knowledge::native::Access> for Access {
    fn from(access: crate::assistant::knowledge::native::Access) -> Self {
        use crate::assistant::knowledge::native::Access as Native;
        match access {
            Native::Granted => Self::Granted,
            Native::NotAsked => Self::NotAsked,
            Native::Denied => Self::Denied,
        }
    }
}

impl From<crate::calendar::CalendarAccess> for Access {
    fn from(access: crate::calendar::CalendarAccess) -> Self {
        use crate::calendar::CalendarAccess;
        match access {
            CalendarAccess::FullAccess => Self::Granted,
            CalendarAccess::NotDetermined => Self::NotAsked,
            _ => Self::Denied,
        }
    }
}

static LAST: [AtomicU8; 4] = [const { AtomicU8::new(0) }; 4];

/// The answer last read for `source`; `Unknown` before.
pub fn last(source: Source) -> Access {
    Access::from_code(LAST[source.index()].load(Ordering::Acquire))
}

fn remember(source: Source, access: Access) -> Access {
    // A grant made in this process can still read "not asked" until Hex
    // relaunches (EventKit); the grant stands until macOS says otherwise.
    let access = if access == Access::NotAsked && last(source) == Access::Granted {
        Access::Granted
    } else {
        access
    };
    LAST[source.index()].store(access.code(), Ordering::Release);
    access
}

/// Reads `source` now, without asking. Quick; off the main thread all the
/// same (Full Disk Access opens a protected file).
pub fn read(source: Source) -> Access {
    if !cfg!(target_os = "macos") || cfg!(test) {
        return Access::Granted;
    }
    remember(source, read_now(source))
}

#[cfg(not(target_os = "macos"))]
fn read_now(_: Source) -> Access {
    Access::Granted
}

#[cfg(not(target_os = "macos"))]
fn request(_: Source) -> Access {
    Access::Granted
}

#[cfg(target_os = "macos")]
fn read_now(source: Source) -> Access {
    use crate::assistant::knowledge::native;
    match source {
        Source::Contacts => native::contacts_access().into(),
        Source::Reminders => native::reminders_access().into(),
        Source::Calendar => crate::calendar::access().into(),
        Source::FullDisk => full_disk_access(),
    }
}

/// Every source, read on its own thread.
pub fn refresh_async() {
    let _ = std::thread::Builder::new()
        .name("data-access-status".into())
        .spawn(|| {
            for source in Source::ALL {
                read(source);
            }
        });
}

/// The sources Allow asks for now: every one macOS prompts for that isn't
/// answered yet (or not read yet), in order. A refusal stays the person's
/// answer: it isn't asked again.
pub fn to_ask(read: impl Fn(Source) -> Access) -> Vec<Source> {
    Source::ASKED
        .into_iter()
        .filter(|source| matches!(read(*source), Access::NotAsked | Access::Unknown))
        .collect()
}

/// Allow: macOS's prompts, one after the other, for [`to_ask`]. Each
/// blocks until answered, so they run on their own thread.
pub fn request_all_async() {
    let sources = to_ask(last);
    if sources.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("data-access-request".into())
        .spawn(move || {
            for source in sources {
                let answer = remember(source, request(source));
                tracing::info!(?source, ?answer, "asked macOS for the assistant's data");
            }
        });
}

/// The ones refused, for "Open System Settings".
pub fn refused(read: impl Fn(Source) -> Access) -> Vec<Source> {
    Source::ASKED
        .into_iter()
        .filter(|source| read(*source) == Access::Denied)
        .collect()
}

/// Shows macOS's prompt; blocks until the person answers.
#[cfg(target_os = "macos")]
fn request(source: Source) -> Access {
    use crate::assistant::knowledge::native;
    match source {
        Source::Contacts => native::request_contacts().into(),
        Source::Reminders => native::request_reminders().into(),
        Source::Calendar => crate::calendar::request_access().access().into(),
        Source::FullDisk => full_disk_access(),
    }
}

/// System Settings › Privacy & Security › `source`'s list.
pub fn open_settings(source: Source) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{}",
                source.settings_pane()
            ))
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = source;
}

/// Full Disk Access: whether macOS lets Hex open TCC's own database, which
/// every Mac has and only Full Disk Access opens.
fn full_disk_access() -> Access {
    let Some(home) = crate::app_paths::home_dir() else {
        return Access::Unknown;
    };
    access_of(&home.join("Library/Application Support/com.apple.TCC/TCC.db"))
}

fn access_of(path: &std::path::Path) -> Access {
    match std::fs::File::open(path) {
        Ok(_) => Access::Granted,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Access::Unknown,
        // Refused: there's no prompt to answer, so it counts as off.
        Err(_) => Access::Denied,
    }
}

/// Whether every source macOS prompts for was answered: what the repair
/// guide and the permissions page wait for. `read` decides how each is
/// read (the real one, or a test's).
pub fn all_answered(read: impl Fn(Source) -> Access) -> bool {
    Source::ASKED
        .into_iter()
        .all(|source| read(source).answered())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_survive_their_code() {
        for access in [
            Access::Granted,
            Access::NotAsked,
            Access::Denied,
            Access::Unknown,
        ] {
            assert_eq!(Access::from_code(access.code()), access);
        }
    }

    #[test]
    fn a_refusal_counts_as_answered_and_only_never_asked_waits() {
        assert!(Access::Granted.answered());
        assert!(Access::Denied.answered());
        assert!(Access::Unknown.answered());
        assert!(!Access::NotAsked.answered());
        assert!(all_answered(|_| Access::Denied));
        assert!(!all_answered(|source| if source == Source::Reminders {
            Access::NotAsked
        } else {
            Access::Granted
        }));
        // Full Disk Access has no prompt: it never holds anything up.
        assert!(all_answered(|source| if source == Source::FullDisk {
            Access::NotAsked
        } else {
            Access::Granted
        }));
    }

    #[test]
    fn allow_asks_only_what_was_never_answered_and_a_refusal_stays() {
        let read = |source| match source {
            Source::Contacts => Access::Granted,
            Source::Reminders => Access::Denied,
            Source::Calendar => Access::NotAsked,
            Source::FullDisk => Access::NotAsked,
        };
        assert_eq!(to_ask(read), [Source::Calendar]);
        assert_eq!(refused(read), [Source::Reminders]);
        // Not read yet: asking is harmless, macOS answers at once if it knows.
        assert_eq!(to_ask(|_| Access::Unknown), Source::ASKED);
        assert!(to_ask(|_| Access::Granted).is_empty());
    }

    #[test]
    fn the_frameworks_answers_read_as_one_kind() {
        use crate::assistant::knowledge::native::Access as Native;
        use crate::calendar::CalendarAccess;
        assert_eq!(Access::from(Native::NotAsked), Access::NotAsked);
        assert_eq!(Access::from(Native::Denied), Access::Denied);
        assert_eq!(Access::from(CalendarAccess::FullAccess), Access::Granted);
        assert_eq!(
            Access::from(CalendarAccess::NotDetermined),
            Access::NotAsked
        );
        // Write-only can't read the agenda: it is off for the assistant.
        assert_eq!(Access::from(CalendarAccess::WriteOnly), Access::Denied);
    }

    #[test]
    fn full_disk_access_is_read_from_a_protected_file() {
        let missing = std::env::temp_dir().join(format!("hex-no-such-{}", std::process::id()));
        assert_eq!(access_of(&missing), Access::Unknown);
        let here = std::env::temp_dir().join(format!("hex-fda-{}", std::process::id()));
        std::fs::write(&here, b"x").unwrap();
        assert_eq!(access_of(&here), Access::Granted);
        let _ = std::fs::remove_file(&here);
    }

    #[test]
    fn each_source_opens_its_own_privacy_list() {
        let panes: Vec<_> = Source::ALL
            .iter()
            .map(|source| source.settings_pane())
            .collect();
        assert_eq!(
            panes,
            [
                "Privacy_Contacts",
                "Privacy_Reminders",
                "Privacy_Calendars",
                "Privacy_AllFiles"
            ]
        );
    }
}
