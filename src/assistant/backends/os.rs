//! Which OS Hex runs on, as a value, so the decisions that differ per OS
//! (where tools live, how they install, what to open) are plain functions
//! of it and tested for every OS on any one; and opening a link, file, or
//! folder the way this OS does.

use std::ffi::OsStr;

/// An OS the assistant runs on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    /// The OS this build runs on (Linux for any other Unix).
    pub const HERE: Os = if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    };

    /// Whether its PATH is read from the person's login shell (a GUI app
    /// doesn't inherit it); Windows keeps PATH in the registry instead.
    pub fn has_login_shell(self) -> bool {
        self != Os::Windows
    }

    /// `PATH`'s separator.
    pub fn path_separator(self) -> char {
        if self == Os::Windows { ';' } else { ':' }
    }

    /// What the person's computer is called, for an agent's prompt: "Mac",
    /// "Windows PC", "Linux computer".
    pub fn computer(self) -> &'static str {
        match self {
            Os::Mac => "Mac",
            Os::Windows => "Windows PC",
            Os::Linux => "Linux computer",
        }
    }

    /// What a person calls the place they paste a command.
    pub fn terminal(self) -> &'static str {
        match self {
            Os::Mac => "Terminal",
            Os::Windows => "PowerShell",
            Os::Linux => "a terminal",
        }
    }
}

/// The program and arguments that open `target` on `os`, where it is a
/// program (Windows opens through the shell's API instead: `None`).
pub fn opener(os: Os) -> Option<(&'static str, &'static [&'static str])> {
    match os {
        Os::Mac => Some(("/usr/bin/open", &[])),
        Os::Linux => Some(("xdg-open", &[])),
        Os::Windows => None,
    }
}

/// Opens a link, file, or folder in its default app, as a double-click
/// would. Returns once it is handed over.
pub fn open(target: impl AsRef<OsStr>) -> std::io::Result<()> {
    let target = target.as_ref();
    let Some((program, args)) = opener(Os::HERE) else {
        #[cfg(windows)]
        return windows_open(target);
        #[cfg(not(windows))]
        return Err(std::io::Error::other("no opener on this OS"));
    };
    let status = crate::desktop::background_process::command(program)
        .args(args)
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "`{program}` couldn't open {}",
            target.to_string_lossy()
        )))
    }
}

#[cfg(windows)]
fn windows_open(target: &OsStr) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |text: &OsStr| -> Vec<u16> { text.encode_wide().chain([0]).collect() };
    let verb = wide(OsStr::new("open"));
    let file = wide(target);
    // SAFETY: both strings are NUL-terminated and live through the call;
    // the other pointers may be null.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Above 32 is success; the rest are error codes.
    if result as usize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "Windows couldn't open {} (error {})",
            target.to_string_lossy(),
            result as usize
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_os_opens_things_its_own_way() {
        assert_eq!(opener(Os::Mac), Some(("/usr/bin/open", &[][..])));
        assert_eq!(opener(Os::Linux), Some(("xdg-open", &[][..])));
        assert_eq!(opener(Os::Windows), None);
        assert!(Os::Mac.has_login_shell() && Os::Linux.has_login_shell());
        assert!(!Os::Windows.has_login_shell());
        assert_eq!(Os::Windows.path_separator(), ';');
    }
}
