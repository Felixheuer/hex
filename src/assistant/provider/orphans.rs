//! Agent processes an Agent-Desk that crashed or was killed left running. Each
//! backend runs as its own process group (`backends::run::own_group`), and
//! a task's stop ends it; but an Agent-Desk that never got to stop it (a crash, a
//! force quit, an update's restart) left the agent working on, with its MCP
//! servers and browsers, beside the job Agent-Desk then resumes. Every group a task
//! starts is noted in `<assistant>/processes.json` with its start time and
//! this Agent-Desk's own; at startup, before jobs resume, [`sweep`] ends the groups
//! whose Agent-Desk is gone (and that are still the very processes noted, not a
//! later one given the same pid).

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    /// The group's leader.
    pid: u32,
    started_us: Option<u64>,
    /// The Agent-Desk that started it.
    hex_pid: u32,
    hex_started_us: Option<u64>,
}

static LOCK: Mutex<()> = Mutex::new(());

fn path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(
        crate::assistant::store::assistant_dir()
            .ok()?
            .join("processes.json"),
    )
}

fn load(path: &std::path::Path) -> Vec<Record> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(path: &std::path::Path, records: &[Record]) {
    if let Ok(bytes) = serde_json::to_vec(records)
        && let Err(error) = crate::durable::write(path, &bytes)
    {
        tracing::debug!(%error, "couldn't note the agent processes");
    }
}

/// A task started process group `pid`.
pub(super) fn note(pid: u32) {
    let Some(path) = path() else {
        return;
    };
    let _held = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut records = load(&path);
    let me = std::process::id();
    records.retain(|record| record.pid != pid);
    records.push(Record {
        pid,
        started_us: crate::assistant::guard::process_started_us(pid),
        hex_pid: me,
        hex_started_us: crate::assistant::guard::process_started_us(me),
    });
    save(&path, &records);
}

/// Process group `pid` ended (or its task let it go).
pub(super) fn forget(pid: u32) {
    let Some(path) = path() else {
        return;
    };
    let _held = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut records = load(&path);
    let before = records.len();
    records.retain(|record| record.pid != pid);
    if records.len() != before {
        save(&path, &records);
    }
}

/// Ends the agent process groups an earlier Agent-Desk left running; how many.
/// Call once at startup, before jobs resume.
pub fn sweep() -> usize {
    let Some(path) = path() else {
        return 0;
    };
    let _held = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let records = load(&path);
    let (left, kept) = orphaned(records, |pid, started| {
        crate::assistant::guard::process_running(pid, started)
    });
    for record in &left {
        tracing::info!(
            pid = record.pid,
            "ending an agent an earlier Agent-Desk left running"
        );
        end(record.pid);
    }
    save(&path, &kept);
    left.len()
}

/// Ends the agent process groups this Agent-Desk started, as it quits: a
/// job's agent and its MCP servers (the guard runs the app's own executable)
/// would otherwise outlive it, and macOS then takes the guard for a running
/// Agent-Desk and refuses to open the app again (-600). Their jobs resume at
/// the next start.
pub fn end_own() -> usize {
    let Some(path) = path() else {
        return 0;
    };
    let _held = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let me = std::process::id();
    let records = load(&path);
    let (mine, others): (Vec<Record>, Vec<Record>) =
        records.into_iter().partition(|record| record.hex_pid == me);
    for record in &mine {
        #[cfg(unix)]
        if let Ok(group) = i32::try_from(record.pid) {
            // SAFETY: signals the process group this Agent-Desk started.
            unsafe { libc::kill(-group, libc::SIGTERM) };
        }
    }
    save(&path, &others);
    mine.len()
}

/// The records whose Agent-Desk is gone and whose group still runs (to end), and
/// those of an Agent-Desk still running (to keep). The rest are forgotten.
fn orphaned(
    records: Vec<Record>,
    running: impl Fn(u32, Option<u64>) -> bool,
) -> (Vec<Record>, Vec<Record>) {
    let mut left = Vec::new();
    let mut kept = Vec::new();
    for record in records {
        if running(record.hex_pid, record.hex_started_us) {
            kept.push(record);
        } else if record.started_us.is_some() && running(record.pid, record.started_us) {
            // Only a process proven to be the one noted: a pid reused by
            // another program is never signalled.
            left.push(record);
        }
    }
    (left, kept)
}

fn end(pid: u32) {
    #[cfg(unix)]
    if let Ok(group) = i32::try_from(pid) {
        // SAFETY: signals the process group this agent led.
        unsafe { libc::kill(-group, libc::SIGTERM) };
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            // SAFETY: as above; a group that ended makes it a no-op.
            unsafe { libc::kill(-group, libc::SIGKILL) };
        });
    }
    #[cfg(not(unix))]
    let _ = pid;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: u32, started: Option<u64>, hex_pid: u32) -> Record {
        Record {
            pid,
            started_us: started,
            hex_pid,
            hex_started_us: Some(1),
        }
    }

    #[test]
    fn only_a_gone_hex_s_own_agents_are_ended() {
        let running = |pid: u32, started: Option<u64>| match pid {
            // This Agent-Desk.
            10 => true,
            // An agent still running, as noted.
            100 => started == Some(5),
            // A pid now used by another program (a later start time).
            200 => started == Some(999),
            _ => false,
        };
        let (left, kept) = orphaned(
            vec![
                record(100, Some(5), 9),  // a dead Agent-Desk's agent: ended
                record(101, Some(5), 10), // this Agent-Desk's agent: kept
                record(200, Some(6), 9),  // reused pid: left alone
                record(300, Some(7), 9),  // already gone: forgotten
                record(100, None, 9),     // no start time: never signalled
            ],
            running,
        );
        assert_eq!(left, [record(100, Some(5), 9)]);
        assert_eq!(kept, [record(101, Some(5), 10)]);
    }
}
