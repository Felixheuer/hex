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
    }
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
