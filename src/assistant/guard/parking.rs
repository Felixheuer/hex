//! Where windows Hex moved out of the person's way came from, so they always
//! go back: one record per window in `parking/<window>.json` (the guard's
//! state dir), shared by every task's guard and by Hex itself (the overlay's
//! Take over and Hand back).
//!
//! A record is created exclusively (`O_EXCL`, under `parking/.lock`): the
//! first park of a window keeps its original frame, and parking it again
//! (after a brief borrow, a Hand back, a move behind the person's windows)
//! never overwrites that frame with a parked one.
//!
//! A window goes back, and its record with it, when the person takes it over
//! ([`unpark`]), or when nobody holds it any more ([`put_back`] with
//! [`lapsed`]): its task ended, or the desk's holder (Hex) quit, crashed, or
//! was updated. Hex puts back lapsed windows when it starts and whenever a
//! task ends, and every parked window when it quits.

use serde::{Deserialize, Serialize};

use super::state::{Store, file_name, json_files, locked_in};
use crate::assistant::space::{Display, off_screen};

/// A frame: x, y, width, height in global points (top-left origin).
pub type Rect = (f64, f64, f64, f64);

/// How a window waits out of the person's way.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// Off the screen but a sliver (`space::parking_spot`).
    OffScreen,
    /// On the person's display, behind their windows (it renders or takes
    /// input only on the screen: `input::decide`).
    Behind,
}

/// A parked window as stored.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParkedWindow {
    pub window_id: String,
    pub app: String,
    /// Its frame before Hex first moved it; `None` when unknown.
    pub from: Option<Rect>,
    pub how: How,
    /// The task it was parked for (the desk's, or a job's).
    pub owner: String,
    pub since_ms: u64,
}

/// Moves windows back: the window server and Accessibility in Hex
/// (`space::Native`), a stand-in in tests.
pub trait Mover {
    /// The displays now.
    fn displays(&mut self) -> Vec<Display>;
    /// Window `id`'s frame now; `None` when it's gone.
    fn frame(&mut self, id: &str) -> Option<Rect>;
    /// Moves `window`'s top-left corner to `to` (restoring it if minimized).
    fn move_to(&mut self, window: &ParkedWindow, to: (f64, f64)) -> Result<(), String>;
}

fn dir(store: &Store) -> std::path::PathBuf {
    store.root().join("parking")
}

fn path(store: &Store, window_id: &str) -> std::path::PathBuf {
    dir(store).join(format!("{}.json", file_name(window_id)))
}

fn read(path: &std::path::Path) -> Option<ParkedWindow> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Notes that window `window_id` of `app`, which was at `from`, is parked
/// `how` for `owner`. A window parked already keeps the frame it had first;
/// only how it waits and for whom change.
pub fn park(
    store: &Store,
    window_id: &str,
    app: &str,
    from: Option<Rect>,
    how: How,
    owner: &str,
    now_ms: u64,
) {
    let dir = dir(store);
    locked_in(&dir, || {
        let path = path(store, window_id);
        if let Some(mut known) = read(&path) {
            if known.how == how && known.owner == owner {
                return;
            }
            known.how = how;
            known.owner = owner.to_owned();
            let temporary = path.with_extension("json.tmp");
            if std::fs::write(&temporary, serde_json::to_vec(&known).unwrap_or_default()).is_ok() {
                let _ = std::fs::rename(temporary, path);
            }
            return;
        }
        let record = ParkedWindow {
            window_id: window_id.to_owned(),
            app: app.to_owned(),
            from,
            how,
            owner: owner.to_owned(),
            since_ms: now_ms,
        };
        if let Err(error) = create_exclusive(&path, &record) {
            tracing::warn!(%error, window = window_id, "couldn't note a parked window");
        }
    });
}

fn create_exclusive(path: &std::path::Path, record: &ParkedWindow) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec(record).map_err(std::io::Error::other)?)?;
    file.sync_all()
}

/// Window `window_id`'s record, if it is parked.
pub fn parked(store: &Store, window_id: &str) -> Option<ParkedWindow> {
    read(&path(store, window_id))
}

/// Takes window `window_id`'s record: it's back with the person (Take over),
/// or going back now.
pub fn unpark(store: &Store, window_id: &str) -> Option<ParkedWindow> {
    locked_in(&dir(store), || {
        let path = path(store, window_id);
        let record = read(&path);
        let _ = std::fs::remove_file(path);
        record
    })
}

/// Every parked window, oldest first.
pub fn all(store: &Store) -> Vec<ParkedWindow> {
    let mut records: Vec<ParkedWindow> = json_files(&dir(store))
        .iter()
        .filter_map(|path| read(path))
        .collect();
    records.sort_by(|a, b| (a.since_ms, &a.window_id).cmp(&(b.since_ms, &b.window_id)));
    records
}

/// The parked windows nobody holds any more (their task ended, or the
/// desk's holder is gone), read under the leases' lock so a window being
/// handed to the desk isn't taken for one.
pub fn lapsed(store: &Store) -> Vec<ParkedWindow> {
    store.locked(|| {
        all(store)
            .into_iter()
            .filter(|record| store.holder(&record.window_id).is_none())
            .collect()
    })
}

/// Where a window of `size` that was at `from` goes back to: there, when
/// enough of it shows on the displays now; else a spot on the main display
/// (its display is gone, or where it was is unknown or itself off the
/// screen).
pub fn back_to(from: Option<Rect>, size: (f64, f64), displays: &[Display]) -> Option<(f64, f64)> {
    if let Some(frame) = from
        && !displays.is_empty()
        && !off_screen(frame, displays)
    {
        return Some((frame.0, frame.1));
    }
    let main = displays
        .iter()
        .find(|display| display.primary)
        .or(displays.first())?;
    // Inside the main display, below the menu bar, as far in as it fits.
    let (width, height) = (size.0.max(0.0), size.1.max(0.0));
    let x = main.x + 80.0_f64.min((main.width - width).max(0.0));
    let y = main.y + 80.0_f64.min((main.height - height).max(25.0));
    Some((x, y))
}

/// Puts `records` back where they came from (`back_to`) and forgets them;
/// a window that's gone is just forgotten, one that didn't move is kept
/// for the next try. How many went back.
pub fn put_back(store: &Store, mover: &mut dyn Mover, records: Vec<ParkedWindow>) -> usize {
    if records.is_empty() {
        return 0;
    }
    let displays = mover.displays();
    let mut back = 0;
    for record in records {
        let Some(frame) = mover.frame(&record.window_id) else {
            unpark(store, &record.window_id);
            continue;
        };
        let Some(to) = back_to(record.from, (frame.2, frame.3), &displays) else {
            continue;
        };
        match mover.move_to(&record, to) {
            Ok(()) => {
                unpark(store, &record.window_id);
                back += 1;
                tracing::info!(
                    window = record.window_id,
                    app = record.app,
                    "a parked window went back where it was"
                );
            }
            Err(error) => tracing::warn!(
                error,
                window = record.window_id,
                "a parked window didn't go back; tried again later"
            ),
        }
    }
    back
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::assistant::guard::desk::DESK;
    use crate::assistant::guard::state::Acquire;

    fn scratch() -> (Store, std::path::PathBuf) {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "hex-parking-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        (Store::new(&path), path)
    }

    fn laptop() -> Display {
        Display {
            id: "1".into(),
            x: 0.0,
            y: 0.0,
            width: 1800.0,
            height: 1169.0,
            primary: true,
        }
    }

    /// Windows by id with their frames; moves are recorded.
    #[derive(Default)]
    struct Fake {
        displays: Vec<Display>,
        frames: HashMap<String, Rect>,
        moved: Vec<(String, (f64, f64))>,
        stuck: bool,
    }

    impl Mover for Fake {
        fn displays(&mut self) -> Vec<Display> {
            self.displays.clone()
        }
        fn frame(&mut self, id: &str) -> Option<Rect> {
            self.frames.get(id).copied()
        }
        fn move_to(&mut self, window: &ParkedWindow, to: (f64, f64)) -> Result<(), String> {
            if self.stuck {
                return Err("-25204".into());
            }
            self.moved.push((window.window_id.clone(), to));
            Ok(())
        }
    }

    #[test]
    fn the_first_park_keeps_the_original_frame() {
        let (store, path) = scratch();
        let original = (200.0, 150.0, 900.0, 600.0);
        park(
            &store,
            "w-1",
            "Mail",
            Some(original),
            How::OffScreen,
            "job",
            1,
        );
        // Parked again from where it hangs (after a borrow): the frame
        // stays the first one; how it waits changes.
        park(
            &store,
            "w-1",
            "Mail",
            Some((1799.0, 1168.0, 900.0, 600.0)),
            How::Behind,
            "job",
            2,
        );
        let record = parked(&store, "w-1").unwrap();
        assert_eq!(record.from, Some(original));
        assert_eq!(record.how, How::Behind);
        assert_eq!(record.since_ms, 1);
        // Taken over: the record goes with it.
        assert_eq!(unpark(&store, "w-1").unwrap().from, Some(original));
        assert!(parked(&store, "w-1").is_none());
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn a_window_goes_back_where_it_was_or_onto_the_main_display() {
        let screens = [laptop()];
        // Where it was shows: there.
        assert_eq!(
            back_to(Some((200.0, 150.0, 900.0, 600.0)), (900.0, 600.0), &screens),
            Some((200.0, 150.0))
        );
        // It was on a display that's gone (left of this one): the main one.
        assert_eq!(
            back_to(
                Some((-1700.0, 100.0, 900.0, 600.0)),
                (900.0, 600.0),
                &screens
            ),
            Some((80.0, 80.0))
        );
        // Unknown, or recorded off the screen itself: the main one, as far
        // in as a big window fits.
        assert_eq!(back_to(None, (1780.0, 600.0), &screens), Some((20.0, 80.0)));
        assert_eq!(
            back_to(
                Some((1799.0, 1168.0, 900.0, 600.0)),
                (900.0, 600.0),
                &screens
            ),
            Some((80.0, 80.0))
        );
        assert_eq!(back_to(None, (900.0, 600.0), &[]), None);
    }

    #[test]
    fn parked_windows_whose_lease_lapsed_go_back_after_hex_ends() {
        let (store, path) = scratch();
        let pid = std::process::id();
        // A desk window (held by this process, standing for Hex) and a
        // job's window whose guard has ended (a pid that isn't running).
        assert_eq!(
            store.acquire(DESK, pid, "w-1", "WhatsApp", 1),
            Acquire::Acquired
        );
        park(
            &store,
            "w-1",
            "WhatsApp",
            Some((100.0, 100.0, 800.0, 600.0)),
            How::OffScreen,
            DESK,
            1,
        );
        park(
            &store,
            "w-2",
            "Mail",
            Some((-1700.0, 100.0, 800.0, 600.0)),
            How::Behind,
            "job",
            2,
        );
        park(&store, "w-3", "Notes", None, How::OffScreen, "job", 3);
        let lapsed_now: Vec<String> = lapsed(&store)
            .into_iter()
            .map(|record| record.window_id)
            .collect();
        // The desk still holds w-1: it stays parked.
        assert_eq!(lapsed_now, ["w-2", "w-3"]);
        let mut mover = Fake {
            displays: vec![laptop()],
            // w-3 has closed meanwhile.
            frames: [("w-2".to_owned(), (80.0, 80.0, 800.0, 600.0))].into(),
            ..Fake::default()
        };
        assert_eq!(put_back(&store, &mut mover, lapsed(&store)), 1);
        // Its display is gone: onto the main one.
        assert_eq!(mover.moved, [("w-2".to_owned(), (80.0, 80.0))]);
        assert!(parked(&store, "w-2").is_none());
        assert!(parked(&store, "w-3").is_none());
        // Hex quits (or crashed and starts again): the desk lapses, and its
        // window goes back where it was.
        store.release(DESK, "w-1");
        mover
            .frames
            .insert("w-1".into(), (1799.0, 1168.0, 800.0, 600.0));
        assert_eq!(put_back(&store, &mut mover, lapsed(&store)), 1);
        assert_eq!(mover.moved[1], ("w-1".to_owned(), (100.0, 100.0)));
        assert!(all(&store).is_empty());
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn a_window_that_did_not_move_is_tried_again() {
        let (store, path) = scratch();
        park(
            &store,
            "w-4",
            "Slack",
            Some((10.0, 40.0, 800.0, 600.0)),
            How::OffScreen,
            "job",
            1,
        );
        let mut mover = Fake {
            displays: vec![laptop()],
            frames: [("w-4".to_owned(), (1799.0, 1168.0, 800.0, 600.0))].into(),
            stuck: true,
            ..Fake::default()
        };
        assert_eq!(put_back(&store, &mut mover, all(&store)), 0);
        assert!(parked(&store, "w-4").is_some());
        mover.stuck = false;
        assert_eq!(put_back(&store, &mut mover, all(&store)), 1);
        assert!(parked(&store, "w-4").is_none());
        let _ = std::fs::remove_dir_all(path);
    }
}
