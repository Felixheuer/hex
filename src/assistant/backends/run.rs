//! Running a helper process with a time limit: its output is read on its own
//! threads, and at the limit it and everything it started (an agent's MCP
//! servers, a login shell's children) are killed as one process group.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// What a bounded run left behind.
#[derive(Debug)]
pub struct Ran {
    /// `None` when it was killed at the limit.
    pub status: Option<ExitStatus>,
    pub stdout: String,
    pub stderr: String,
    pub took: Duration,
}

impl Ran {
    pub fn success(&self) -> bool {
        self.status.is_some_and(|status| status.success())
    }

    pub fn timed_out(&self) -> bool {
        self.status.is_none()
    }

    /// The last non-empty line of stderr, else of stdout, for an error.
    pub fn last_words(&self) -> String {
        [&self.stderr, &self.stdout]
            .into_iter()
            .find_map(|text| {
                text.lines()
                    .map(str::trim)
                    .rfind(|line| !line.is_empty())
                    .map(|line| shorten(line, 240))
            })
            .unwrap_or_default()
    }
}

/// Runs `command` with stdin closed, killing its process group after
/// `limit`. Fails only when it can't start.
pub fn bounded(mut command: Command, limit: Duration) -> std::io::Result<Ran> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    own_group(&mut command, true);
    let started = Instant::now();
    let mut child = command.spawn()?;
    let tree = Tree::of(&child);
    let (done, finished) = mpsc::channel();
    let stdout = reader(child.stdout.take(), done.clone());
    let stderr = reader(child.stderr.take(), done);
    let status = wait_until(&mut child, started + limit);
    // Something it left running in the background (a login shell's agent)
    // can hold its output open: wait briefly, then end the group.
    let grace = Instant::now() + Duration::from_millis(if status.is_some() { 500 } else { 0 });
    let mut open = 2;
    while open > 0 {
        let left = grace.saturating_duration_since(Instant::now());
        match finished.recv_timeout(left) {
            Ok(()) => open -= 1,
            Err(_) => break,
        }
    }
    if status.is_none() || open > 0 {
        tree.kill_with(&mut child);
    }
    Ok(Ran {
        status,
        stdout: take(&stdout),
        stderr: take(&stderr),
        took: started.elapsed(),
    })
}

/// [`bounded`], handing each line of output to `line` as it comes (a
/// progress bar's `\r`-separated updates count as lines), for an install's
/// progress.
pub fn streamed(
    mut command: Command,
    limit: Duration,
    line: &mut dyn FnMut(&str),
) -> std::io::Result<Ran> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    own_group(&mut command, true);
    let started = Instant::now();
    let deadline = started + limit;
    let mut child = command.spawn()?;
    let tree = Tree::of(&child);
    let (sender, lines) = mpsc::channel();
    split_lines(child.stdout.take(), false, sender.clone());
    split_lines(child.stderr.take(), true, sender);
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let mut status = None;
    let mut exited_at = None;
    loop {
        let wait = Duration::from_millis(50);
        match lines.recv_timeout(wait) {
            Ok((is_err, text)) => {
                line(&text);
                let all = if is_err { &mut stderr } else { &mut stdout };
                all.push_str(&text);
                all.push('\n');
                continue;
            }
            // Both streams closed.
            Err(mpsc::RecvTimeoutError::Disconnected) if status.is_some() => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(wait),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = child.try_wait().ok().flatten();
            if status.is_some() {
                exited_at = Some(Instant::now());
            }
        }
        let lingering = exited_at.is_some_and(|at| at.elapsed() > Duration::from_millis(500));
        if lingering || (status.is_none() && Instant::now() >= deadline) {
            break;
        }
    }
    if status.is_none() || exited_at.is_some_and(|at| at.elapsed() > Duration::from_millis(500)) {
        tree.kill_with(&mut child);
    }
    Ok(Ran {
        status,
        stdout,
        stderr,
        took: started.elapsed(),
    })
}

/// Reads `stream` on its own thread, sending each `\n`- or `\r`-ended,
/// non-blank piece.
fn split_lines(
    stream: Option<impl Read + Send + 'static>,
    is_err: bool,
    lines: mpsc::Sender<(bool, String)>,
) {
    std::thread::spawn(move || {
        let Some(mut stream) = stream else { return };
        let mut pending = Vec::new();
        let mut chunk = [0_u8; 4096];
        let send = |bytes: &mut Vec<u8>| {
            let text = String::from_utf8_lossy(bytes).trim().to_owned();
            bytes.clear();
            text.is_empty() || lines.send((is_err, text)).is_ok()
        };
        while let Ok(read @ 1..) = stream.read(&mut chunk) {
            for &byte in &chunk[..read] {
                if byte == b'\n' || byte == b'\r' {
                    if !send(&mut pending) {
                        return;
                    }
                } else {
                    pending.push(byte);
                }
            }
        }
        send(&mut pending);
    });
}

type Buffer = Arc<Mutex<Vec<u8>>>;

fn reader(stream: Option<impl Read + Send + 'static>, done: mpsc::Sender<()>) -> Buffer {
    let buffer = Buffer::default();
    let filling = buffer.clone();
    std::thread::spawn(move || {
        if let Some(mut stream) = stream {
            let mut chunk = [0_u8; 8192];
            while let Ok(read @ 1..) = stream.read(&mut chunk) {
                filling
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .extend_from_slice(&chunk[..read]);
            }
        }
        let _ = done.send(());
    });
    buffer
}

fn take(buffer: &Buffer) -> String {
    let bytes = buffer.lock().unwrap_or_else(|error| error.into_inner());
    String::from_utf8_lossy(&bytes).into_owned()
}

fn wait_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(15));
            }
            _ => return None,
        }
    }
}

/// Kills `child` and the process group it leads, then reaps it. Off Unix
/// it ends `child` alone; keep a [`Tree`] to reach its children there.
pub fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: Signals the group this child leads (`process_group(0)`).
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Starts `command` as the head of its own process tree: its own process
/// group on Unix (a [`Tree`] made after spawning reaches it). With
/// `with_thread`, on Linux the direct child also dies with the thread that
/// spawns it, so a caller that waits for it never leaves it behind; leave it
/// off when another thread keeps the process (a prewarmed agent).
pub fn own_group(command: &mut Command, with_thread: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        #[cfg(target_os = "linux")]
        if with_thread {
            // SAFETY: `prctl` is async-signal-safe and touches only the child.
            unsafe {
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    Ok(())
                });
            }
        }
    }
    let _ = (command, with_thread);
}

/// Everything one started process began: its process group on Unix
/// ([`own_group`]), a job object on Windows that ends them all if Hex stops
/// without cleaning up. Dropping it leaves what still runs alone (a daemon
/// the program started on purpose), as on Unix; [`Tree::kill`] ends it all.
#[derive(Clone)]
pub struct Tree {
    pid: u32,
    #[cfg(windows)]
    job: Option<Arc<Released>>,
}

/// A job released on its last drop.
#[cfg(windows)]
struct Released(crate::personal_commands::process::windows_job::Job);

#[cfg(windows)]
impl Drop for Released {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl std::fmt::Debug for Tree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tree").field("pid", &self.pid).finish()
    }
}

impl Tree {
    /// The tree `child` heads; spawn it after [`own_group`].
    pub fn of(child: &Child) -> Self {
        #[cfg(windows)]
        {
            use crate::personal_commands::process::windows_job::Job;
            let job = Job::new().and_then(|job| job.assign(child).map(|()| job));
            if let Err(error) = &job {
                tracing::debug!(%error, "couldn't put an agent in a job; only it can be ended");
            }
            Self {
                pid: child.id(),
                job: job.ok().map(|job| Arc::new(Released(job))),
            }
        }
        #[cfg(not(windows))]
        Self { pid: child.id() }
    }

    /// The head process's id.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Asks the tree to stop: SIGTERM on Unix. Windows has nothing gentler
    /// than ending the job.
    pub fn terminate(&self) {
        #[cfg(unix)]
        self.signal(libc::SIGTERM);
        #[cfg(windows)]
        self.end();
    }

    /// Ends the tree at once.
    pub fn kill(&self) {
        #[cfg(unix)]
        self.signal(libc::SIGKILL);
        #[cfg(windows)]
        self.end();
    }

    /// [`Tree::kill`], then reaps `child`.
    pub fn kill_with(&self, child: &mut Child) {
        self.kill();
        let _ = child.kill();
        let _ = child.wait();
    }

    #[cfg(unix)]
    fn signal(&self, signal: libc::c_int) {
        if let Ok(group) = i32::try_from(self.pid)
            && group > 0
        {
            // SAFETY: a negative pid signals the group this tree's head leads.
            unsafe { libc::kill(-group, signal) };
        }
    }

    #[cfg(windows)]
    fn end(&self) {
        match &self.job {
            Some(job) => job.0.terminate(),
            None => terminate_pid(self.pid),
        }
    }
}

/// Ends process `pid` (a process no job holds).
#[cfg(windows)]
fn terminate_pid(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    // SAFETY: the handle is checked, used once, and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

/// `text` cut to `max` characters, with an ellipsis when cut.
pub fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_quick_command_reports_its_output() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "echo out; echo err >&2"]);
        let ran = bounded(command, Duration::from_secs(5)).unwrap();
        assert!(ran.success());
        assert_eq!(ran.stdout.trim(), "out");
        assert_eq!(ran.last_words(), "err");
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_command_and_its_children_are_killed_at_the_limit() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & sleep 30; echo never"]);
        let ran = bounded(command, Duration::from_millis(200)).unwrap();
        assert!(ran.timed_out());
        assert!(ran.took < Duration::from_secs(5));
        assert!(!ran.stdout.contains("never"));
    }

    #[cfg(unix)]
    #[test]
    fn a_child_left_holding_the_output_does_not_hold_the_caller() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & echo hi"]);
        let ran = bounded(command, Duration::from_secs(10)).unwrap();
        assert!(ran.success());
        assert_eq!(ran.stdout.trim(), "hi");
        assert!(ran.took < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn progress_comes_line_by_line_as_it_happens() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "echo one; printf '10%%\\r50%%\\r'; echo err >&2; echo two",
        ]);
        let mut seen = Vec::new();
        let ran = streamed(command, Duration::from_secs(5), &mut |line| {
            seen.push(line.to_owned())
        })
        .unwrap();
        assert!(ran.success());
        seen.sort();
        assert_eq!(seen, ["10%", "50%", "err", "one", "two"]);
        assert_eq!(ran.last_words(), "err");
    }

    #[cfg(unix)]
    #[test]
    fn a_streamed_command_is_killed_at_the_limit() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "echo started; sleep 30"]);
        let mut seen = Vec::new();
        let ran = streamed(command, Duration::from_millis(300), &mut |line| {
            seen.push(line.to_owned())
        })
        .unwrap();
        assert!(ran.timed_out());
        assert!(ran.took < Duration::from_secs(5));
        assert_eq!(seen, ["started"]);
    }

    #[test]
    fn long_text_is_shortened_on_a_character_boundary() {
        assert_eq!(shorten("äöü", 2), "äö…");
        assert_eq!(shorten("ok", 2), "ok");
    }
}
