//! Running one task on a backend: `claude -p --output-format stream-json`,
//! `codex exec --json`, `opencode run --format json`, or the Claude Messages
//! API, each given agent-desktop as its desktop tool, with their output turned
//! into `Report`s.
//!
//! Every backend process runs in its own process group, so cancelling a task
//! ends the agent and everything it started (its agent-desktop MCP server,
//! any command it ran) together.
//!
//! OWNER: brain.

mod claude_api;
mod claude_code;
mod codex;
pub mod desktop;
mod mcp;
mod opencode;

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};

use crate::app_settings::AssistantBackend;
use crate::assistant::types::{Message, TaskWindow};

/// The agent-desktop a task may use, and who it is there.
#[derive(Clone, Debug)]
pub struct Desktop {
    pub executable: PathBuf,
    /// The Agent mode multi-agent session, when one is running.
    pub session: Option<String>,
    /// This task's id within the session; its own cursor.
    pub agent_id: String,
    /// Real pointer and keyboard input (`--headed`) is allowed.
    pub headed: bool,
}

impl Desktop {
    /// The global flags for every agent-desktop call this task makes.
    pub fn flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if let Some(session) = &self.session {
            flags.extend(["--session".to_owned(), session.clone()]);
            flags.extend(["--agent-id".to_owned(), self.agent_id.clone()]);
        }
        if self.headed {
            flags.push("--headed".into());
        }
        flags
    }

    /// The environment that names the session and agent, for every process
    /// the task starts.
    pub fn env(&self) -> Vec<(String, String)> {
        let mut env = vec![("AGENT_DESKTOP_AGENT_ID".to_owned(), self.agent_id.clone())];
        if let Some(session) = &self.session {
            env.push(("AGENT_DESKTOP_SESSION".into(), session.clone()));
        }
        env
    }

    /// `agent-desktop [flags] mcp`, as a program and its arguments.
    pub fn mcp_command(&self) -> (String, Vec<String>) {
        let mut args = self.flags();
        args.push("mcp".into());
        (self.executable.display().to_string(), args)
    }
}

/// agent-desktop's MCP tools a task never gets: they manage the shared
/// session and every agent's cursor, or block in stateless mode.
pub const WITHHELD_TOOLS: &[&str] = &[
    "desktop_session_start",
    "desktop_session_end",
    "desktop_session_list",
    "desktop_session_gc",
    "desktop_cursor_overlay_enable",
    "desktop_cursor_overlay_disable",
    "desktop_trace_show",
    "desktop_trace_export",
    "desktop_key_down",
    "desktop_key_up",
    "desktop_mouse_down",
    "desktop_mouse_up",
    "desktop_permissions",
];

/// One task for a backend.
#[derive(Clone, Debug)]
pub struct Run {
    pub backend: AssistantBackend,
    /// The backend's model; `None` is its own default.
    pub model: Option<String>,
    pub system: String,
    /// The message to send: the question, with a quick question's context.
    pub prompt: String,
    /// A screenshot to show the model with the prompt.
    pub image: Option<PathBuf>,
    /// The backend's own session to continue.
    pub resume: Option<String>,
    /// The conversation so far, for the Claude API, which keeps no session.
    pub history: Vec<Message>,
    /// `None`: answer only, no desktop tools.
    pub desktop: Option<Desktop>,
    /// The working folder the agent runs in.
    pub workdir: PathBuf,
    /// A quick question: answer fast and short.
    pub quick: bool,
}

/// What a running backend reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    /// Waiting on the model.
    Thinking,
    /// One thing on the desktop: what is being done, and how the transcript
    /// records it once done.
    Step {
        doing: String,
        done: String,
    },
    Window(TaskWindow),
    /// More answer text.
    Delta(String),
    /// The backend's session id, to continue the conversation later.
    Session(String),
}

/// How a backend finished.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub text: String,
}

/// A task's stop switch and the process groups it started.
#[derive(Debug, Default)]
pub struct Control {
    cancelled: AtomicBool,
    groups: Mutex<Vec<u32>>,
}

impl Control {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Stops the task: ends every process group it started.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let groups = self
            .groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for group in groups {
            end_group(group);
        }
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn track(&self, pid: u32) {
        self.groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(pid);
        if self.cancelled() {
            end_group(pid);
        }
    }

    fn untrack(&self, pid: u32) {
        self.groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|group| *group != pid);
    }
}

/// Runs `run` on its backend until it answers, fails, or is cancelled.
pub fn run(run: &Run, control: &Control, report: &mut dyn FnMut(Report)) -> Result<Finished> {
    match run.backend {
        AssistantBackend::ClaudeCode => claude_code::run(run, control, report),
        AssistantBackend::Codex => codex::run(run, control, report),
        AssistantBackend::OpenCode => opencode::run(run, control, report),
        AssistantBackend::ClaudeApi => claude_api::run(run, control, report),
    }
}

/// The person-readable name of `backend`.
pub fn name(backend: AssistantBackend) -> &'static str {
    match backend {
        AssistantBackend::ClaudeCode => "Claude Code",
        AssistantBackend::Codex => "Codex",
        AssistantBackend::OpenCode => "OpenCode",
        AssistantBackend::ClaudeApi => "the Claude API",
    }
}

/// The key a conversation saves `backend`'s session under.
pub fn session_key(backend: AssistantBackend) -> &'static str {
    match backend {
        AssistantBackend::ClaudeCode => "claude_code",
        AssistantBackend::Codex => "codex",
        AssistantBackend::OpenCode => "open_code",
        AssistantBackend::ClaudeApi => "claude_api",
    }
}

/// The executable for `backend`: what setup found, else a search of PATH and
/// the folders installers use.
pub fn executable(backend: AssistantBackend) -> Result<PathBuf> {
    let program = match backend {
        AssistantBackend::ClaudeCode => "claude",
        AssistantBackend::Codex => "codex",
        AssistantBackend::OpenCode => "opencode",
        AssistantBackend::ClaudeApi => return Err(eyre!("the Claude API has no executable")),
    };
    crate::assistant::backends::executable(backend)
        .or_else(|| find_program(program))
        .ok_or_else(|| {
            eyre!(
                "{} isn't installed on this Mac, or Hex can't find it. Install it, or choose another agent in Labs › Assistant.",
                name(backend)
            )
        })
}

/// agent-desktop, as setup found it or on PATH.
pub fn agent_desktop() -> Option<PathBuf> {
    crate::assistant::backends::agent_desktop()
        .path
        .or_else(|| find_program("agent-desktop"))
}

/// `program` on PATH, in `~/.local/bin`, or where Homebrew puts it.
pub fn find_program(program: &str) -> Option<PathBuf> {
    search_path()
        .into_iter()
        .map(|dir| dir.join(program))
        .find(|path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && is_executable(&meta))
        })
}

fn search_path() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = crate::app_paths::home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".opencode/bin"));
        dirs.push(home.join(".bun/bin"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    let mut unique = Vec::new();
    for dir in dirs {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    unique
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    true
}

/// A command for `executable` in `workdir`, its PATH led by the
/// executable's own folder (a Node-based CLI finds its `node` there) and the
/// usual install folders, and the desktop's session in its environment.
pub fn command(executable: &Path, workdir: &Path, desktop: Option<&Desktop>) -> Command {
    let mut command = crate::desktop::background_process::command(executable);
    command.current_dir(workdir);
    let mut dirs: Vec<PathBuf> = executable
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    dirs.extend(search_path());
    if let Ok(path) = std::env::join_paths(dirs) {
        command.env("PATH", path);
    }
    for (key, value) in desktop.map(Desktop::env).unwrap_or_default() {
        command.env(key, value);
    }
    command
}

/// What a process gets on its standard input.
pub enum Stdin {
    Null,
    /// These bytes, then end of input.
    Write(Vec<u8>),
    /// A pipe the caller keeps writing to (`Process::stdin`).
    Keep,
}

/// A running backend process: its stdout read line by line, its stderr's
/// end kept for errors.
pub struct Process {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<BufReader<std::process::ChildStdout>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    stderr_thread: Option<std::thread::JoinHandle<()>>,
}

impl Process {
    /// Starts `command` in its own process group.
    pub fn spawn(mut command: Command, stdin: Stdin, control: &Control) -> Result<Self> {
        if control.cancelled() {
            return Err(eyre!("cancelled"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
            .stdin(if matches!(stdin, Stdin::Null) {
                Stdio::null()
            } else {
                Stdio::piped()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let program = command.get_program().to_owned();
        let mut child = command
            .spawn()
            .map_err(|error| eyre!("couldn't start {}: {error}", show(&program)))?;
        control.track(child.id());
        let mut kept = None;
        match (stdin, child.stdin.take()) {
            (Stdin::Write(data), Some(mut pipe)) => {
                std::thread::spawn(move || {
                    let _ = std::io::Write::write_all(&mut pipe, &data);
                });
            }
            (Stdin::Keep, pipe) => kept = pipe,
            _ => {}
        }
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_thread = child.stderr.take().map(|pipe| {
            let tail = Arc::clone(&stderr);
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(|line| line.ok()) {
                    let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                    tail.push_back(line);
                    if tail.len() > 40 {
                        tail.pop_front();
                    }
                }
            })
        });
        let stdout = child.stdout.take().map(BufReader::new);
        Ok(Self {
            child,
            stdin: kept,
            stdout,
            stderr,
            stderr_thread,
        })
    }

    /// The input pipe of a process started with `Stdin::Keep`.
    pub fn stdin(&mut self) -> Option<&mut std::process::ChildStdin> {
        self.stdin.as_mut()
    }

    /// The next line of output, until it ends.
    pub fn next_line(&mut self) -> Option<String> {
        let mut line = String::new();
        match self.stdout.as_mut()?.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_owned()),
        }
    }

    /// Waits for the process to exit; its status and the end of its stderr.
    pub fn finish(mut self, control: &Control) -> (Option<ExitStatus>, String) {
        drop(self.stdin.take());
        let status = self.child.wait().ok();
        control.untrack(self.child.id());
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
        let stderr = self
            .stderr
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|line| !line.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        (status, stderr)
    }
}

/// Runs `executable args` to completion (an agent-desktop call) and returns
/// its stdout, ended early when the task is cancelled.
pub fn output(command: Command, control: &Control, limit: usize) -> Result<String> {
    let mut process = Process::spawn(command, Stdin::Null, control)?;
    let mut text = String::new();
    if let Some(reader) = process.stdout.take() {
        let mut buffer = Vec::new();
        let _ = reader.take(limit as u64).read_to_end(&mut buffer);
        text = String::from_utf8_lossy(&buffer).into_owned();
    }
    let (status, stderr) = process.finish(control);
    if control.cancelled() {
        return Err(eyre!("cancelled"));
    }
    if text.trim().is_empty() && !status.is_some_and(|status| status.success()) {
        return Err(eyre!("{}", last_line(&stderr, "the command failed")));
    }
    Ok(text)
}

/// The last non-empty line of `text`, or `fallback`.
pub fn last_line(text: &str, fallback: &str) -> String {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// Why a backend that ended without an answer failed, for a person.
pub fn exit_error(
    backend: AssistantBackend,
    status: Option<ExitStatus>,
    stderr: &str,
) -> color_eyre::Report {
    let detail = last_line(stderr, "");
    match (status.and_then(|status| status.code()), detail.is_empty()) {
        (_, false) => eyre!("{} stopped: {detail}", name(backend)),
        (Some(code), true) => eyre!(
            "{} stopped with exit code {code} and no answer",
            name(backend)
        ),
        (None, true) => eyre!("{} stopped without an answer", name(backend)),
    }
}

fn show(program: &OsString) -> String {
    Path::new(program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string_lossy().into_owned())
}

/// Sends SIGTERM to process group `group`, then SIGKILL a moment later.
fn end_group(group: u32) {
    #[cfg(unix)]
    {
        let Ok(group) = i32::try_from(group) else {
            return;
        };
        // SAFETY: kill only sends a signal to the group the task started.
        unsafe {
            libc::kill(-group, libc::SIGTERM);
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            // SAFETY: as above; a group that already ended makes this a no-op.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        });
    }
    #[cfg(not(unix))]
    let _ = group;
}

/// Standard base64 of `data`, for images sent inline.
pub fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(TABLE[(n >> shift & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }

    #[test]
    fn desktop_flags_name_the_session_only_when_there_is_one() {
        let mut desktop = Desktop {
            executable: "/opt/homebrew/bin/agent-desktop".into(),
            session: None,
            agent_id: "hex-1".into(),
            headed: false,
        };
        assert!(desktop.flags().is_empty());
        desktop.session = Some("s1".into());
        desktop.headed = true;
        assert_eq!(
            desktop.flags(),
            ["--session", "s1", "--agent-id", "hex-1", "--headed"]
        );
        assert_eq!(desktop.mcp_command().1.last().unwrap(), "mcp");
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_ends_the_whole_process_group() {
        let control = Control::new();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & sleep 30; echo done"]);
        let mut process = Process::spawn(command, Stdin::Null, &control).unwrap();
        let started = std::time::Instant::now();
        control.cancel();
        assert_eq!(process.next_line(), None);
        let (status, _) = process.finish(&control);
        assert!(!status.unwrap().success());
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
