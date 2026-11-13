//! Holding a taken-over task's shell work. When the person takes a task
//! over, its guard holds every desktop action and its browser waits, but a
//! shell command (a build, `git push`, a script editing files) would go on
//! under their hands. [`stop`] sends SIGSTOP to the shell commands below the
//! backend's process; [`resume`] sends SIGCONT to exactly those on hand back.
//!
//! The backend process itself is never stopped: its connection to the model
//! must stay alive, and it only waits on the stopped command anyway. What
//! counts as shell work is every process tree below it that starts with a
//! shell or `sandbox-exec` (how Claude Code, Codex, and OpenCode run their
//! shell tool); the MCP servers it starts directly keep running (the guard
//! and the browser hold themselves).

/// One running process as `ps` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub parent: u32,
    /// Its command's name or path.
    pub command: String,
}

/// The programs a shell tool runs its commands in.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "fish", "sandbox-exec"];

fn is_shell(command: &str) -> bool {
    let name = command.rsplit('/').next().unwrap_or(command);
    let name = name.strip_prefix('-').unwrap_or(name);
    SHELLS.contains(&name)
}

/// The shell work below `root` in `processes`: each subtree that starts
/// with a shell, whole; never `root` itself.
pub fn shell_work(processes: &[Process], root: u32) -> Vec<u32> {
    let mut found = Vec::new();
    // (pid, inside shell work)
    let mut queue = vec![(root, false)];
    let mut seen = std::collections::HashSet::from([root]);
    while let Some((pid, inside)) = queue.pop() {
        for child in processes.iter().filter(|process| process.parent == pid) {
            if !seen.insert(child.pid) {
                continue;
            }
            let held = inside || is_shell(&child.command);
            if held {
                found.push(child.pid);
            }
            queue.push((child.pid, held));
        }
    }
    found.sort_unstable();
    found
}

/// Reads `ps -A -o pid=,ppid=,comm=` output.
pub fn parse_ps(output: &str) -> Vec<Process> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let parent = parts.next()?.parse().ok()?;
            let command = parts.collect::<Vec<_>>().join(" ");
            Some(Process {
                pid,
                parent,
                command,
            })
        })
        .collect()
}

/// Stops the shell work below each of `roots`; the pids it stopped.
pub fn stop(roots: &[u32]) -> Vec<u32> {
    let Some(processes) = list() else {
        return Vec::new();
    };
    let pids: Vec<u32> = roots
        .iter()
        .flat_map(|root| shell_work(&processes, *root))
        .collect();
    signal(&pids, Signal::Stop);
    if !pids.is_empty() {
        tracing::info!(
            count = pids.len(),
            "taken over: holding the task's shell commands"
        );
    }
    pids
}

/// Lets `pids` (from [`stop`]) run again.
pub fn resume(pids: &[u32]) {
    signal(pids, Signal::Continue);
}

#[derive(Clone, Copy)]
enum Signal {
    Stop,
    Continue,
}

#[cfg(all(unix, not(test)))]
fn list() -> Option<Vec<Process>> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,ppid=,comm="])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_ps(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(any(not(unix), test))]
fn list() -> Option<Vec<Process>> {
    None
}

#[cfg(unix)]
fn signal(pids: &[u32], signal: Signal) {
    let signal = match signal {
        Signal::Stop => libc::SIGSTOP,
        Signal::Continue => libc::SIGCONT,
    };
    for pid in pids {
        if let Ok(pid) = i32::try_from(*pid)
            && pid > 1
        {
            // SAFETY: signals one process this task started.
            unsafe { libc::kill(pid, signal) };
        }
    }
}

#[cfg(not(unix))]
fn signal(_pids: &[u32], _signal: Signal) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, parent: u32, command: &str) -> Process {
        Process {
            pid,
            parent,
            command: command.into(),
        }
    }

    #[test]
    fn only_shell_commands_below_the_backend_are_held() {
        let processes = vec![
            process(10, 1, "/usr/local/bin/claude"),
            // Its MCP servers keep running.
            process(
                11,
                10,
                "/Applications/Agent-Desk.app/Contents/MacOS/agent-desk",
            ),
            process(12, 11, "cua-driver"),
            // A shell tool's command, with what it started.
            process(20, 10, "/bin/zsh"),
            process(21, 20, "cargo"),
            process(22, 21, "rustc"),
            // A sandboxed one.
            process(30, 10, "/usr/bin/sandbox-exec"),
            process(31, 30, "-bash"),
            process(32, 31, "git"),
            // Someone else's shell.
            process(40, 1, "/bin/zsh"),
        ];
        assert_eq!(shell_work(&processes, 10), vec![20, 21, 22, 30, 31, 32]);
        assert!(shell_work(&processes, 99).is_empty(), "gone: nothing");
    }

    #[test]
    fn ps_output_is_read_with_paths_and_spaces() {
        let processes = parse_ps(
            "  10     1 /usr/local/bin/claude\n  20    10 /Applications/Visual Studio Code.app/x\nbad\n",
        );
        assert_eq!(processes.len(), 2);
        assert_eq!(processes[1].parent, 10);
        assert_eq!(
            processes[1].command,
            "/Applications/Visual Studio Code.app/x"
        );
    }
}
