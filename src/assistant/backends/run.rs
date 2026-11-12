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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let started = Instant::now();
    let mut child = command.spawn()?;
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
        kill_group(&mut child);
    }
    Ok(Ran {
        status,
        stdout: take(&stdout),
        stderr: take(&stderr),
        took: started.elapsed(),
    })
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

/// Kills `child` and the process group it leads, then reaps it.
pub fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: Signals the group this child leads (`process_group(0)`).
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
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

    #[test]
    fn a_quick_command_reports_its_output() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "echo out; echo err >&2"]);
        let ran = bounded(command, Duration::from_secs(5)).unwrap();
        assert!(ran.success());
        assert_eq!(ran.stdout.trim(), "out");
        assert_eq!(ran.last_words(), "err");
    }

    #[test]
    fn a_slow_command_and_its_children_are_killed_at_the_limit() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & sleep 30; echo never"]);
        let ran = bounded(command, Duration::from_millis(200)).unwrap();
        assert!(ran.timed_out());
        assert!(ran.took < Duration::from_secs(5));
        assert!(!ran.stdout.contains("never"));
    }

    #[test]
    fn a_child_left_holding_the_output_does_not_hold_the_caller() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & echo hi"]);
        let ran = bounded(command, Duration::from_secs(10)).unwrap();
        assert!(ran.success());
        assert_eq!(ran.stdout.trim(), "hi");
        assert!(ran.took < Duration::from_secs(5));
    }

    #[test]
    fn long_text_is_shortened_on_a_character_boundary() {
        assert_eq!(shorten("äöü", 2), "äö…");
        assert_eq!(shorten("ok", 2), "ok");
    }
}
