//! Running one task on a backend: `claude -p --output-format stream-json`,
//! `codex exec --json`, `opencode run --format json`, or the Claude Messages
//! API, each given its desktop tool (the guard's MCP server, with Cua Driver
//! behind it) and, for web work, its own hidden browser (`servers`), with
//! their output turned into `Report`s.
//!
//! Claude Code can be started ahead (`warm`): its process and MCP servers
//! come up while the person still speaks, and a Here conversation's process
//! stays for the follow-up.
//!
//! Every backend process runs in its own process group, so cancelling a task
//! ends the agent and everything it started (its MCP servers, the desktop
//! driver behind the guard, any command it ran) together.
//!
//! OWNER: brain.

mod claude_api;
mod claude_code;
mod codex;
pub mod desktop;
pub mod hold;
pub mod limit;
mod mcp;
mod opencode;
pub mod orphans;
pub mod servers;
mod warm;

pub use claude_api::classify_reply;

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
use crate::assistant::backends::McpServer;
use crate::assistant::backends::run::{self, Tree};
use crate::assistant::types::{Message, TaskWindow};

/// The desktop tool a task may use.
#[derive(Clone, Debug)]
pub struct Desktop {
    /// The guard's MCP server for this task: a task never reaches the
    /// desktop driver (Cua Driver) without it.
    pub server: McpServer,
    /// Real pointer and keyboard input is allowed.
    pub headed: bool,
    /// A Background task may also operate the window in front
    /// (`assistant.full_control`; the guard enforces it).
    pub front_window: bool,
    /// A Here task may open and use apps in windows of its own, parked out
    /// of the person's way (the guard enforces it).
    pub open_apps: bool,
}

impl Desktop {
    /// A task's guard at a made-up path, for tests.
    #[cfg(test)]
    pub fn fixture() -> Self {
        Self::new(McpServer {
            command: "/Applications/Agent-Desk.app/Contents/MacOS/agent-desk".into(),
            args: vec![
                "assistant".into(),
                "mcp".into(),
                "--spec".into(),
                r#"{"task":"t","lane":"background"}"#.into(),
            ],
            path_env: "/usr/bin:/bin".into(),
        })
    }

    /// The desktop tool for a task: `guard`'s server, with Cua Driver
    /// behind it (`guard::mcp_server`).
    pub fn new(guard: McpServer) -> Self {
        Self {
            server: guard,
            headed: false,
            front_window: false,
            open_apps: false,
        }
    }

    /// Whether the task opens and uses any app (full control, or allowed to
    /// open apps in windows of its own); a Background task always does.
    pub fn reaches_apps(&self) -> bool {
        self.open_apps || self.front_window
    }
}

/// One task for a backend.
#[derive(Clone, Debug)]
pub struct Run {
    pub backend: AssistantBackend,
    /// The backend's model; `None` is its own default.
    pub model: Option<String>,
    /// How hard the model thinks (Claude Code's `--effort`); `None` is its
    /// default.
    pub effort: Option<String>,
    /// Claude Code stops the turn with [`Escalate`] once this many tool
    /// calls have answered and none is still running, so the strong model
    /// goes on in the same session (`agent`). `None` never stops it.
    pub escalate_after: Option<usize>,
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
    /// The task's own hidden browser (`browser::mcp_server`), if any.
    pub browser: Option<McpServer>,
    /// The working folder the agent runs in.
    pub workdir: PathBuf,
    /// A quick question: answer fast and short.
    pub quick: bool,
    /// Keep the backend's process for a follow-up this long after it
    /// answers (Claude Code only); `None` ends it.
    pub keep_warm: Option<Duration>,
    /// A job's folder (also `workdir`): the backend gets file tools and a
    /// sandboxed shell that may write only there.
    pub workspace: Option<PathBuf>,
    /// Agent-Desk's own tools (`assistant::tools`): memory and schedules.
    pub tools: Option<McpServer>,
    /// A job in Yolo mode: no sandbox and no permission prompts. Its shell
    /// runs as the person, with the whole network and every folder they can
    /// reach (git, gh, npm, tailscale, brew, …).
    pub unrestricted: bool,
    /// The backend's whole environment when it needs more than Agent-Desk's usual
    /// one (OpenCode with a bring-your-own-key provider: the key by env).
    pub backend_env: Option<Vec<(String, String)>>,
    /// More OpenCode configuration (a bring-your-own-key provider).
    pub provider_config: Option<serde_json::Value>,
}

impl Run {
    /// A run with only the essentials, for tests.
    #[cfg(test)]
    pub fn fixture(backend: AssistantBackend) -> Self {
        Self {
            backend,
            model: None,
            effort: None,
            escalate_after: None,
            system: "You are Agent-Desk.".into(),
            prompt: "hi".into(),
            image: None,
            resume: None,
            history: Vec::new(),
            desktop: None,
            browser: None,
            workdir: std::env::temp_dir(),
            quick: false,
            keep_warm: None,
            workspace: None,
            tools: None,
            unrestricted: false,
            backend_env: None,
            provider_config: None,
        }
    }
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
    /// A tool the agent calls, for the audit log (`assistant::audit`): its
    /// name and its arguments, compact with secrets blanked.
    Tool {
        name: String,
        input: String,
    },
    /// The backend said something (a stream line) that shows nothing: a
    /// sign it still works, for a quick turn's watchdog
    /// (`agent::watchdog`).
    Alive,
    /// A step ended with no tool running: the session's size now, for a
    /// long job's fresh session (`agent::rollover`).
    Usage(crate::assistant::agent::rollover::Usage),
}

impl Report {
    /// A call of `name` with `input`, for the audit log.
    pub fn tool(name: &str, input: &serde_json::Value) -> Self {
        Self::Tool {
            name: name.to_owned(),
            input: crate::assistant::audit::compact(input),
        }
    }
}

/// How a backend finished.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub text: String,
}

/// A fast model's turn stopped after [`Run::escalate_after`] tool calls:
/// the strong model goes on in its session. The error a run returns then.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Escalate {
    /// The tool calls it made.
    pub tools: usize,
}

impl std::fmt::Display for Escalate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the fast model handed over after {} tool calls",
            self.tools
        )
    }
}

impl std::error::Error for Escalate {}

/// Whether `error` is a fast model's hand-over ([`Escalate`]).
pub fn escalated(error: &color_eyre::Report) -> Option<Escalate> {
    error.downcast_ref::<Escalate>().copied()
}

/// A task's stop switch and the process groups it started.
#[derive(Debug, Default)]
pub struct Control {
    cancelled: AtomicBool,
    /// Stopped to take a new instruction (steering), not to end.
    steered: AtomicBool,
    /// Stopped by Agent-Desk for this reason (it ran too long): a failure.
    failure: Mutex<Option<String>>,
    /// Turns ended for making no progress (`stall`).
    stalls: std::sync::atomic::AtomicUsize,
    /// Sessions ended to go on in a fresh one (`rotate`).
    rotations: std::sync::atomic::AtomicUsize,
    groups: Mutex<Vec<Tree>>,
    /// The shell commands held while the person took the task over.
    held: Mutex<Vec<u32>>,
}

impl Control {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Stops the task: ends every process group it started.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.hold(false);
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

    /// Holds (taken over) or lets go (handed back) the shell commands this
    /// task's backend runs (`hold`); the backend itself keeps running.
    pub fn hold(&self, on: bool) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        if on {
            if held.is_empty() {
                let roots: Vec<u32> = self
                    .groups
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .map(Tree::pid)
                    .collect();
                *held = hold::stop(&roots);
            }
        } else {
            hold::resume(&std::mem::take(&mut *held));
        }
    }

    /// Stops the current turn because the person gave a new instruction:
    /// the conversation goes on with it.
    pub fn steer(&self) {
        self.steered.store(true, Ordering::Release);
        self.cancel();
    }

    pub fn steered(&self) -> bool {
        self.steered.load(Ordering::Acquire)
    }

    /// Stops the task as failed, for `reason` (it ran too long).
    pub fn fail(&self, reason: &str) {
        *self.failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason.to_owned());
        self.cancel();
    }

    /// Ends the current turn's processes because it made no progress for
    /// too long (`agent::watchdog`); unlike [`cancel`](Self::cancel) the
    /// task goes on: the strong model takes the turn over, or it fails.
    pub fn stall(&self) {
        self.stalls.fetch_add(1, Ordering::AcqRel);
        let groups = self
            .groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for group in groups {
            end_group(group);
        }
    }

    /// Ends the current turn's processes so a long job goes on in a fresh
    /// session (`agent::rollover`); like [`stall`](Self::stall) the task
    /// goes on.
    pub fn rotate(&self) {
        self.rotations.fetch_add(1, Ordering::AcqRel);
        let groups = self
            .groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for group in groups {
            end_group(group);
        }
    }

    /// How often [`rotate`](Self::rotate) ended a turn of this task.
    pub fn rotations(&self) -> usize {
        self.rotations.load(Ordering::Acquire)
    }

    /// How often [`stall`](Self::stall) ended a turn of this task.
    pub fn stalls(&self) -> usize {
        self.stalls.load(Ordering::Acquire)
    }

    /// Why Agent-Desk stopped the task as failed, if it did.
    pub fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn track(&self, tree: &Tree) {
        orphans::note(tree.pid());
        self.groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tree.clone());
        if self.cancelled() {
            end_group(tree.clone());
        }
    }

    fn untrack(&self, pid: u32) {
        orphans::forget(pid);
        self.groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|group| group.pid() != pid);
    }
}

/// Starts `run`'s backend ahead of its question (Claude Code), so the
/// process and its MCP servers are up when the question comes. It waits at
/// most `ttl`.
pub fn prewarm(run: &Run, ttl: Duration) {
    if run.backend == AssistantBackend::ClaudeCode {
        claude_code::prewarm(run, ttl);
    }
}

/// Ends every backend process started ahead.
pub fn cool_down() {
    warm::clear();
}

/// Ends the backend processes started ahead for `task` (a reservation the
/// request it was made for didn't take), instead of letting them idle
/// until their time is up.
pub fn end_ahead(task: &str) {
    warm::end_for(task);
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

/// The key a conversation saves `backend`'s session under for `lane`.
///
/// Each lane keeps its own: a session carries its lane's rules and tools
/// (a Background one its hidden browsers and "never the window in front", a
/// Here one only that window), so a hold that continues a Background
/// conversation open in Chats must not resume its session, nor a typed
/// follow-up a Here one. The other lane's turn starts a new session that is
/// told the conversation so far.
pub fn session_key(backend: AssistantBackend, lane: crate::assistant::Lane) -> String {
    let backend = match backend {
        AssistantBackend::ClaudeCode => "claude_code",
        AssistantBackend::Codex => "codex",
        AssistantBackend::OpenCode => "open_code",
        AssistantBackend::ClaudeApi => "claude_api",
    };
    match lane {
        crate::assistant::Lane::Background => backend.to_owned(),
        crate::assistant::Lane::Here => format!("{backend}.here"),
    }
}

/// Whether Claude Code can sandbox a job's shell on this computer; why not
/// when it can't (a job then gets no shell).
pub fn claude_code_sandbox() -> std::result::Result<(), String> {
    claude_code::sandbox()
}

/// The executable for `backend`, as `backends` finds it.
pub fn executable(backend: AssistantBackend) -> Result<PathBuf> {
    crate::assistant::backends::executable(backend).ok_or_else(|| {
        eyre!(
            "{} isn't installed on this {}, or Agent-Desk can't find it. Install it, or choose another agent in {}.",
            name(backend),
            crate::assistant::agent::computer_name(),
            crate::assistant::lines::ASSISTANT_SETTINGS
        )
    })
}

/// A command for `executable` in `workdir`, with the environment Agent-Desk
/// gives agents (`backends::agent_command`: its PATH finds `node`).
pub fn command(executable: &Path, workdir: &Path) -> Command {
    let mut command = crate::assistant::backends::agent_command(executable);
    command.current_dir(workdir);
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
    /// It and what it started (its MCP servers), for Cancel.
    tree: Tree,
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
        // Not tied to this thread: a prewarmed agent outlives the one that
        // started it.
        run::own_group(&mut command, false);
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
        let tree = Tree::of(&child);
        control.track(&tree);
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
            tree,
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

/// Runs `executable args` to completion and returns
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

/// Asks `tree` to stop (SIGTERM on Unix), then kills it a moment later. On
/// Windows its job ends at once.
fn end_group(tree: Tree) {
    tree.terminate();
    #[cfg(unix)]
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        // A group that already ended makes this a no-op.
        tree.kill();
    });
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
    fn steering_cancels_the_turn() {
        let control = Control::new();
        control.steer();
        assert!(control.cancelled() && control.steered());
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

    #[test]
    fn each_lane_keeps_its_own_session() {
        use crate::assistant::Lane;
        assert_eq!(
            session_key(AssistantBackend::ClaudeCode, Lane::Background),
            "claude_code",
            "saved Background sessions keep resuming"
        );
        assert_eq!(
            session_key(AssistantBackend::ClaudeCode, Lane::Here),
            "claude_code.here"
        );
        assert_ne!(
            session_key(AssistantBackend::Codex, Lane::Here),
            session_key(AssistantBackend::Codex, Lane::Background)
        );
    }
}
