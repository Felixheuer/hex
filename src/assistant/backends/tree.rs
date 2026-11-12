//! A helper process and everything it starts (an agent's MCP servers, an
//! installer's downloads), ended as one: a process group on macOS and
//! Linux, a job object on Windows, where a process's children outlive it.

use std::process::{Child, Command};

/// Makes the process `command` starts lead a group of its own, so
/// [`Tree::stop`] reaches its children (macOS and Linux; Windows uses the
/// job [`Tree::of`] puts it in).
pub fn own_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// A started process with its children.
#[derive(Debug)]
pub struct Tree {
    pid: u32,
    #[cfg(windows)]
    job: Option<job::Job>,
}

impl Tree {
    /// `child`'s tree. Its command went through [`own_group`].
    pub fn of(child: &Child) -> Self {
        Self {
            pid: child.id(),
            #[cfg(windows)]
            job: job::Job::new()
                .and_then(|job| job.assign(child).map(|()| job))
                .inspect_err(|error| {
                    tracing::debug!(%error, "a helper's children won't end with it");
                })
                .ok(),
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Asks the whole tree to end (SIGTERM); Windows has no asking, so
    /// there it ends at once.
    pub fn stop(&self) {
        #[cfg(unix)]
        self.signal(libc::SIGTERM);
        #[cfg(windows)]
        self.kill();
    }

    /// Ends the whole tree now.
    pub fn kill(&self) {
        #[cfg(unix)]
        self.signal(libc::SIGKILL);
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
    }

    #[cfg(unix)]
    fn signal(&self, signal: libc::c_int) {
        if let Ok(group) = i32::try_from(self.pid) {
            // SAFETY: a negative pid signals the group this child leads
            // (`own_group`); a group that ended makes it a no-op.
            unsafe { libc::kill(-group, signal) };
        }
    }
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject,
    };

    /// A job object holding a helper and the processes it starts. Closing
    /// it leaves them running (an OpenCode service Hex started must
    /// outlive the command that started it); only [`Job::terminate`] ends
    /// them.
    #[derive(Debug)]
    pub struct Job(HANDLE);

    // SAFETY: a job handle may be used and closed from any thread.
    unsafe impl Send for Job {}
    // SAFETY: as above; the job functions used here are thread-safe.
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> std::io::Result<Self> {
            // SAFETY: null attributes and name create an anonymous job.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Self(handle))
        }

        pub fn assign(&self, child: &Child) -> std::io::Result<()> {
            // SAFETY: both handles are open for the duration of the call.
            if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) {
            // SAFETY: the handle is open until drop.
            unsafe { TerminateJobObject(self.0, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is open and closed only here.
            unsafe { CloseHandle(self.0) };
        }
    }
}
