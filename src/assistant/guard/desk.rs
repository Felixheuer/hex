//! The desk: the app windows the assistant opened for the person in a quick
//! (Here) turn. A turn's own leases end with it; these are handed to the
//! desk instead, whose holder is Hex itself (its pid, from the spec), so
//! the window stays parked and shows as a live mini at the orb until the
//! person lets it go (× on the mini), the assistant does
//! (`hex_release_window`, "schließ WhatsApp"), or the window closes. Every
//! later task that may open apps acts in a desk window directly, as if it
//! held it.
//!
//! The desk is leases like any other (`leases/<window>.json`, task
//! [`DESK`]), so a desk window is never claimed by a job meanwhile, and a
//! desk Hex no longer runs lapses with it.
//!
//! Acting in a desk window takes a short, exclusive borrow of it
//! (`desk-borrows/<window>.json`): one task at a time, returned when its
//! turn or job ends ([`return_borrows`]), when its guard ends, or after
//! [`BORROW_IDLE`] without use. A quick (Here) turn borrows as it acts, and
//! takes a window from a job (the person asked now); a job borrows only by
//! claiming the window, and is refused while a quick turn has it.

use std::collections::HashSet;

use super::state::Store;
use crate::assistant::types::TaskWindow;

/// The desk's task id in the leases.
pub const DESK: &str = "desk";

/// The windows on the desk now, oldest first. Windows that have closed are
/// let go on the way.
pub fn desk() -> Vec<TaskWindow> {
    let Some(store) = Store::standard() else {
        return Vec::new();
    };
    desk_in(&store, existing_windows().as_ref())
}

/// Lets go of desk window `window_id` (the person's ×): the mini goes, and
/// the window comes back on the screen (`app_live`).
pub fn desk_release(window_id: &str) {
    if let Some(store) = Store::standard() {
        store.release(DESK, window_id);
        return_borrow(&store, window_id);
    }
}

/// How long a borrow of a desk window lasts without use.
pub const BORROW_IDLE: std::time::Duration = std::time::Duration::from_secs(120);

/// A borrow of a desk window, as stored.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct Borrow {
    pub task: String,
    /// The borrower's guard process, and when it started.
    pub pid: u32,
    #[serde(default)]
    pub started: Option<u64>,
    /// A quick (Here) turn's.
    pub here: bool,
    /// When it was last used.
    pub used_ms: u64,
}

impl Borrow {
    fn live(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.used_ms) < BORROW_IDLE.as_millis() as u64
            && super::state::running(self.pid, self.started)
    }
}

/// What asking to borrow a desk window came to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Lend {
    /// It's this task's to use now (taken, renewed, or taken from a job by
    /// a quick turn).
    Lent,
    /// Another task has it: its id, and whether it's a quick turn.
    Busy { task: String, here: bool },
}

fn borrows_dir(store: &Store) -> std::path::PathBuf {
    store.root().join("desk-borrows")
}

fn borrow_path(store: &Store, window_id: &str) -> std::path::PathBuf {
    borrows_dir(store).join(format!("{}.json", super::state::file_name(window_id)))
}

fn read_borrow(path: &std::path::Path) -> Option<Borrow> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Borrows desk window `window_id` for `task` (a quick turn when `here`),
/// in guard process `pid`: free, lapsed, or its own already, it is lent
/// (and its time renewed). A quick turn always gets it (the person asked
/// now, and only one quick turn runs at a time); a job's is busy while
/// another task has it.
pub(super) fn borrow(
    store: &Store,
    window_id: &str,
    task: &str,
    pid: u32,
    here: bool,
    now_ms: u64,
) -> Lend {
    super::state::locked_in(&borrows_dir(store), || {
        let path = borrow_path(store, window_id);
        if let Some(held) = read_borrow(&path).filter(|held| held.live(now_ms))
            && held.task != task
            && !here
        {
            return Lend::Busy {
                task: held.task,
                here: held.here,
            };
        }
        let lent = Borrow {
            task: task.to_owned(),
            pid,
            started: super::state::started_us(pid),
            here,
            used_ms: now_ms,
        };
        let temporary = path.with_extension("json.tmp");
        let written = std::fs::write(&temporary, serde_json::to_vec(&lent).unwrap_or_default())
            .and_then(|()| std::fs::rename(&temporary, &path));
        if let Err(error) = written {
            tracing::warn!(%error, window = window_id, "couldn't note a desk borrow");
        }
        Lend::Lent
    })
}

/// Who has desk window `window_id` borrowed now, if anyone does.
pub(super) fn borrower(store: &Store, window_id: &str, now_ms: u64) -> Option<Borrow> {
    read_borrow(&borrow_path(store, window_id)).filter(|held| held.live(now_ms))
}

/// Returns `task`'s borrows (its turn or job ended).
pub(super) fn return_borrows(store: &Store, task: &str) {
    super::state::locked_in(&borrows_dir(store), || {
        for path in super::state::json_files(&borrows_dir(store)) {
            if read_borrow(&path).is_some_and(|held| held.task == task) {
                let _ = std::fs::remove_file(path);
            }
        }
    });
}

/// Forgets any borrow of `window_id` (it left the desk).
pub(super) fn return_borrow(store: &Store, window_id: &str) {
    super::state::locked_in(&borrows_dir(store), || {
        let _ = std::fs::remove_file(borrow_path(store, window_id));
    });
}

/// Lets go of every desk window of `app` (by name, as `app_key` compares).
pub fn desk_release_app(app: &str) -> usize {
    Store::standard().map_or(0, |store| release_app_in(&store, app))
}

/// The quick turns whose browsers stay on the desk (task ids, oldest
/// first): `<support>/assistant/desk-browsers.json`.
pub fn desk_browsers() -> Vec<String> {
    Store::standard()
        .map(|store| browsers_in(&store))
        .unwrap_or_default()
}

/// Keeps quick turn `task`'s browsers on the desk (`browser::close`).
pub fn desk_keep_browser(task: &str) {
    if let Some(store) = Store::standard() {
        let mut tasks = browsers_in(&store);
        if !tasks.iter().any(|known| known == task) {
            tasks.push(task.to_owned());
            write_browsers(&store, &tasks);
        }
    }
}

/// Lets go of `task`'s browsers on the desk (they close: `browser::close_now`).
pub fn desk_release_browser(task: &str) {
    if let Some(store) = Store::standard() {
        let mut tasks = browsers_in(&store);
        let before = tasks.len();
        tasks.retain(|known| known != task);
        if tasks.len() != before {
            write_browsers(&store, &tasks);
        }
    }
}

fn browsers_file(store: &Store) -> std::path::PathBuf {
    store.root().join("desk-browsers.json")
}

pub(super) fn browsers_in(store: &Store) -> Vec<String> {
    std::fs::read(browsers_file(store))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_browsers(store: &Store, tasks: &[String]) {
    let path = browsers_file(store);
    let _ = std::fs::create_dir_all(store.root());
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, serde_json::to_vec(tasks).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(temporary, path);
    }
}

pub(super) fn desk_in(store: &Store, existing: Option<&HashSet<u32>>) -> Vec<TaskWindow> {
    let mut windows = Vec::new();
    for lease in store.held_by(DESK) {
        let number = crate::assistant::computer::window_number(&lease.window_id);
        let gone = existing.is_some_and(|existing| number.is_none_or(|n| !existing.contains(&n)));
        if gone {
            store.release(DESK, &lease.window_id);
            continue;
        }
        windows.push(TaskWindow {
            app: lease.app,
            window_id: Some(lease.window_id),
        });
    }
    windows
}

pub(super) fn release_app_in(store: &Store, app: &str) -> usize {
    let key = super::policy::app_key(app);
    let mut released = 0;
    for lease in store.held_by(DESK) {
        if super::policy::app_key(&lease.app) == key {
            store.release(DESK, &lease.window_id);
            return_borrow(store, &lease.window_id);
            released += 1;
        }
    }
    released
}

/// Every window the window server has (any desktop), by number; `None`
/// where it can't be read (then nothing is pruned).
fn existing_windows() -> Option<HashSet<u32>> {
    if cfg!(not(target_os = "macos")) {
        return None;
    }
    let all = crate::assistant::computer::now::all_windows();
    (!all.is_empty()).then(|| all.into_iter().map(|window| window.number).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::guard::state::Acquire;

    fn scratch() -> (Store, std::path::PathBuf) {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "hex-desk-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        (Store::new(&path), path)
    }

    #[test]
    fn a_turns_window_handed_to_the_desk_outlives_the_turn() {
        let (store, path) = scratch();
        let pid = std::process::id();
        assert_eq!(
            store.acquire("turn", pid, "w-900", "\u{200e}WhatsApp", 1),
            Acquire::Acquired
        );
        assert!(store.hand_over("w-900", "turn", DESK, pid, 2));
        // The turn ends: its leases go, the desk's stays.
        store.release_all("turn");
        let desk = desk_in(&store, None);
        assert_eq!(desk.len(), 1);
        assert_eq!(desk[0].window_id.as_deref(), Some("w-900"));
        // Another task can't take it while the desk holds it.
        assert_eq!(
            store.acquire("job", pid, "w-900", "WhatsApp", 3),
            Acquire::Taken(DESK.into())
        );
        // A window that closed is let go.
        let existing: HashSet<u32> = [1, 2].into_iter().collect();
        assert!(desk_in(&store, Some(&existing)).is_empty());
        assert!(store.held_by(DESK).is_empty());
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn a_desk_window_is_lent_to_one_task_at_a_time() {
        let (store, path) = scratch();
        let pid = std::process::id();
        // A job claims it; another job is refused while it has it.
        assert_eq!(borrow(&store, "w-1", "job", pid, false, 1_000), Lend::Lent);
        assert_eq!(
            borrow(&store, "w-1", "other-job", pid, false, 2_000),
            Lend::Busy {
                task: "job".into(),
                here: false
            }
        );
        // The person's quick turn takes it from the job.
        assert_eq!(borrow(&store, "w-1", "turn", pid, true, 3_000), Lend::Lent);
        // While the turn has it, the job's claim is refused.
        assert_eq!(
            borrow(&store, "w-1", "job", pid, false, 4_000),
            Lend::Busy {
                task: "turn".into(),
                here: true
            }
        );
        assert_eq!(borrower(&store, "w-1", 4_000).unwrap().task, "turn");
        // The turn ends: its borrow comes back, and the job may claim it.
        return_borrows(&store, "turn");
        assert!(borrower(&store, "w-1", 5_000).is_none());
        assert_eq!(borrow(&store, "w-1", "job", pid, false, 5_000), Lend::Lent);
        // A borrow unused for a while lapses.
        let later = 5_000 + BORROW_IDLE.as_millis() as u64;
        assert!(borrower(&store, "w-1", later).is_none());
        assert_eq!(
            borrow(&store, "w-1", "other-job", pid, false, later),
            Lend::Lent
        );
        // So does one whose guard has ended.
        let mut ended = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let dead = ended.id();
        ended.wait().unwrap();
        assert_eq!(borrow(&store, "w-2", "gone", dead, true, 1), Lend::Lent);
        assert_eq!(borrow(&store, "w-2", "job", pid, false, 2), Lend::Lent);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn letting_go_of_an_app_frees_its_desk_windows_only() {
        let (store, path) = scratch();
        let pid = std::process::id();
        store.acquire(DESK, pid, "w-1", "\u{200e}WhatsApp", 1);
        store.acquire(DESK, pid, "w-2", "WhatsApp", 1);
        store.acquire(DESK, pid, "w-3", "Notes", 1);
        // "schließ WhatsApp": both WhatsApp windows, whatever their marks.
        assert_eq!(release_app_in(&store, "whatsapp"), 2);
        let left: Vec<String> = desk_in(&store, None)
            .into_iter()
            .filter_map(|window| window.window_id)
            .collect();
        assert_eq!(left, ["w-3"]);
        // Only a lease the giver holds is handed over.
        assert!(!store.hand_over("w-3", "turn", DESK, pid, 2));
        let _ = std::fs::remove_dir_all(path);
    }
}
