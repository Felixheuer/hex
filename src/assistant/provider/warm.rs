//! Backend processes kept ready: started ahead while the person still
//! speaks (`prewarm`), or kept after a Here answer for the follow-up. Each
//! waits for its next message on stdin; a process is only reused for a run
//! with exactly the same arguments (system prompt, MCP servers with the
//! task's id, model) and the same backend session, so it can't carry one
//! task's tools into another. A Here task's window isn't among them: it goes
//! through the guard's state (`guard::mcp_server`), so a process started in
//! one window serves a request asked in another. Each ends after its time
//! to live, when evicted, when the reservation it was started for goes
//! unused (`end_for`), or when Hex ends (its input closes).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use super::{Process, end_group};

/// At most this many processes wait at once.
const MAX: usize = 3;

struct Warm {
    id: u64,
    key: String,
    session: Option<String>,
    process: Process,
}

static POOL: Mutex<Vec<Warm>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn pool() -> MutexGuard<'static, Vec<Warm>> {
    POOL.lock().unwrap_or_else(|error| error.into_inner())
}

/// Keeps `process` (started with arguments `key`, on backend `session`) for
/// `ttl`.
pub(super) fn put(key: String, session: Option<String>, process: Process, ttl: Duration) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let evicted: Vec<Warm> = {
        let mut pool = pool();
        let mut evicted = Vec::new();
        let mut index = 0;
        while index < pool.len() {
            if pool[index].key == key && pool[index].session == session {
                evicted.push(pool.remove(index));
            } else {
                index += 1;
            }
        }
        pool.push(Warm {
            id,
            key,
            session,
            process,
        });
        while pool.len() > MAX {
            evicted.push(pool.remove(0));
        }
        evicted
    };
    evicted.into_iter().for_each(end);
    std::thread::Builder::new()
        .name("assistant-warm".into())
        .spawn(move || {
            std::thread::sleep(ttl);
            let expired = {
                let mut pool = pool();
                pool.iter()
                    .position(|warm| warm.id == id)
                    .map(|index| pool.remove(index))
            };
            if let Some(warm) = expired {
                end(warm);
            }
        })
        .ok();
}

/// Whether a live process waits for `key` and `session`.
pub(super) fn has(key: &str, session: Option<&str>) -> bool {
    pool()
        .iter()
        .any(|warm| warm.key == key && warm.session.as_deref() == session)
}

/// The waiting process for `key` and `session`, if one is still alive.
pub(super) fn take(key: &str, session: Option<&str>) -> Option<Process> {
    let mut found = {
        let mut pool = pool();
        let index = pool
            .iter()
            .position(|warm| warm.key == key && warm.session.as_deref() == session)?;
        pool.remove(index)
    };
    match found.process.child.try_wait() {
        Ok(None) => Some(found.process),
        _ => {
            end(found);
            None
        }
    }
}

/// Ends the waiting processes started for `task` (a conversation id,
/// which its arguments name: its tools' and guard's `--task`).
pub(super) fn end_for(task: &str) {
    if task.is_empty() {
        return;
    }
    let ended: Vec<Warm> = {
        let mut pool = pool();
        let mut ended = Vec::new();
        let mut index = 0;
        while index < pool.len() {
            if pool[index].key.contains(task) {
                ended.push(pool.remove(index));
            } else {
                index += 1;
            }
        }
        ended
    };
    ended.into_iter().for_each(end);
}

/// Ends every waiting process.
pub(super) fn clear() {
    let all: Vec<Warm> = pool().drain(..).collect();
    all.into_iter().for_each(end);
}

fn end(warm: Warm) {
    let mut process = warm.process;
    drop(process.stdin.take());
    end_group(process.tree.clone());
    std::thread::spawn(move || {
        let _ = process.child.wait();
    });
}
