//! The Contacts and Reminders reads in `knowledge.swift`. They only read
//! when macOS already granted access and never ask: the permission is
//! asked for in onboarding and its repair guide (`desktop::data_access`),
//! never by a tool call.

/// macOS's answer for a kind of data, as the frameworks report it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    Granted,
    NotAsked,
    Denied,
}

#[cfg(target_os = "macos")]
mod bridge {
    use std::ffi::{CStr, CString, c_char};

    unsafe extern "C" {
        fn hex_knowledge_contacts_status() -> i32;
        fn hex_knowledge_contacts(query: *const c_char, limit: i32) -> *mut c_char;
        fn hex_knowledge_reminders_status() -> i32;
        fn hex_knowledge_reminders(query: *const c_char, limit: i32) -> *mut c_char;
        fn hex_knowledge_free(string: *mut c_char);
        fn hex_knowledge_contacts_request() -> i32;
        fn hex_knowledge_reminders_request() -> i32;
    }

    pub fn request_contacts() -> i32 {
        // SAFETY: Blocks on the framework's completion; no arguments.
        unsafe { hex_knowledge_contacts_request() }
    }

    pub fn request_reminders() -> i32 {
        // SAFETY: Blocks on the framework's completion; no arguments.
        unsafe { hex_knowledge_reminders_request() }
    }

    fn take(pointer: *mut c_char) -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        // SAFETY: The bridge returned a NUL-terminated string it allocated.
        let text = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: Freed once, by the allocator that made it.
        unsafe { hex_knowledge_free(pointer) };
        Some(text)
    }

    pub fn contacts_status() -> i32 {
        // SAFETY: A status query without arguments.
        unsafe { hex_knowledge_contacts_status() }
    }

    pub fn reminders_status() -> i32 {
        // SAFETY: A status query without arguments.
        unsafe { hex_knowledge_reminders_status() }
    }

    pub fn contacts(query: &str, limit: i32) -> Option<String> {
        let query = CString::new(query.replace('\0', "")).ok()?;
        // SAFETY: `query` outlives the call; the result is owned or null.
        take(unsafe { hex_knowledge_contacts(query.as_ptr(), limit) })
    }

    pub fn reminders(query: &str, limit: i32) -> Option<String> {
        let query = CString::new(query.replace('\0', "")).ok()?;
        // SAFETY: `query` outlives the call; the result is owned or null.
        take(unsafe { hex_knowledge_reminders(query.as_ptr(), limit) })
    }
}

#[cfg(not(target_os = "macos"))]
mod bridge {
    pub fn contacts_status() -> i32 {
        2
    }
    pub fn reminders_status() -> i32 {
        2
    }
    pub fn request_contacts() -> i32 {
        2
    }
    pub fn request_reminders() -> i32 {
        2
    }
    pub fn contacts(_: &str, _: i32) -> Option<String> {
        None
    }
    pub fn reminders(_: &str, _: i32) -> Option<String> {
        None
    }
}

/// CNAuthorizationStatus: 3 authorized, 4 limited.
pub fn contacts_access() -> Access {
    contacts_code(bridge::contacts_status())
}

/// EKAuthorizationStatus: 3 full access.
pub fn reminders_access() -> Access {
    reminders_code(bridge::reminders_status())
}

/// Shows macOS's Contacts prompt if it was never answered, and returns the
/// answer. Blocks until the person answers: onboarding only, off the main
/// thread.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn request_contacts() -> Access {
    contacts_code(bridge::request_contacts())
}

/// The same for Reminders (full access).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn request_reminders() -> Access {
    reminders_code(bridge::request_reminders())
}

fn contacts_code(code: i32) -> Access {
    match code {
        3 | 4 => Access::Granted,
        0 => Access::NotAsked,
        _ => Access::Denied,
    }
}

fn reminders_code(code: i32) -> Access {
    match code {
        3 => Access::Granted,
        0 => Access::NotAsked,
        _ => Access::Denied,
    }
}

/// Contacts holding every word of `query`, as JSON; `None` without access.
pub fn contacts(query: &str, limit: usize) -> Option<String> {
    bridge::contacts(query, limit.min(500) as i32)
}

/// Open reminders holding every word of `query`, as JSON.
pub fn reminders(query: &str, limit: usize) -> Option<String> {
    bridge::reminders(query, limit.min(500) as i32)
}
