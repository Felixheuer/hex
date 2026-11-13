//! Read-only calls beside the guard's own (#36). The MCP server holds one
//! guard behind a lock, and an action may keep it for seconds: `hex_open_app`
//! waits for the app's window, a real-input action for the pointer
//! (`HANDS_WAIT`), a paused task until it resumes. Reads (snapshots, finds,
//! window pictures, the installed apps, the displays, what an app offers)
//! don't wait behind that: each runs in a reader of its own ([`Seed`]),
//! checked against the task's leases as they are then, sharing the driver
//! (locked per call only), the screen, and what results taught (refs'
//! windows and labels, element spots, pictures) with the guard.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Map, Value};

use super::super::child::{Desktop, Window};
use super::super::screen::{App, Display, MenuKey, ReturnTarget, Screen};
use super::super::state::Store;
use super::{Config, Guard, Origin, consent, vision};

/// One value the guard and its readers share, locked only while used.
pub struct Shared<T: ?Sized>(Arc<Mutex<T>>);

impl<T: ?Sized> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }
}

impl<T: ?Sized> Shared<T> {
    /// The value; a reader that panicked leaves it as it was.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// The driver, one call at a time (it speaks over one pipe).
impl<D: Desktop> Desktop for Shared<D> {
    fn tools(&mut self) -> Result<Vec<Value>, String> {
        self.lock().tools()
    }

    fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String> {
        self.lock().call(tool, args)
    }

}

/// The screen, one look at a time ([`Screen`]'s, locked per call).
impl Shared<Box<dyn Screen>> {
    pub fn windows(&self) -> Vec<Window> {
        self.lock().windows()
    }

    pub fn installed_apps(&self) -> Vec<App> {
        self.lock().installed_apps()
    }

    pub fn running_apps(&self) -> Vec<std::path::PathBuf> {
        self.lock().running_apps()
    }

    pub fn displays(&self) -> Vec<Display> {
        self.lock().displays()
    }

    pub fn off_screen_windows(&self) -> Vec<Window> {
        self.lock().off_screen_windows()
    }

    pub fn app_pids(&self, bundle: &std::path::Path) -> Vec<i64> {
        self.lock().app_pids(bundle)
    }

    pub fn menu_keys(&self, pid: i64) -> Vec<MenuKey> {
        self.lock().menu_keys(pid)
    }

    pub fn return_target(&self, pid: i64) -> ReturnTarget {
        self.lock().return_target(pid)
    }
}

/// What results taught the guard, shared with its readers.
#[derive(Clone)]
pub(super) struct Learned {
    /// Which window each snapshot's refs point into.
    pub snapshots: Shared<HashMap<String, Origin>>,
    /// Labels, roles, and secure fields by ref.
    pub elements: Shared<HashMap<String, consent::Element>>,
    /// Element spots and window pictures.
    pub vision: Shared<vision::Vision>,
}

impl Default for Learned {
    fn default() -> Self {
        Self {
            snapshots: Shared::new(HashMap::new()),
            elements: Shared::new(HashMap::new()),
            vision: Shared::new(vision::Vision::default()),
        }
    }
}

/// Runs an installed app's capability read.
pub type Discover = dyn Fn(&App) -> Value + Send;

/// What a reader starts from: the guard's task, driver, screen, and what it
/// learned. Taken once ([`Guard::seed`]); every read makes a reader of its
/// own, so reads run side by side.
pub struct Seed<D: Desktop> {
    config: Config,
    store: Store,
    pid: u32,
    desktop: Shared<D>,
    screen: Shared<Box<dyn Screen>>,
    learned: Learned,
    discover: Shared<Box<Discover>>,
    desk: Option<u32>,
    fresh_for: Duration,
    window_wait: Duration,
}

impl<D: Desktop> Guard<D> {
    /// The seed of this guard's readers. Its capability reads (`discover`)
    /// are shared from now on.
    pub fn seed(&mut self) -> Seed<D> {
        let discover = Shared::new(std::mem::replace(
            &mut self.discover,
            Box::new(|_: &App| Value::Null),
        ));
        let shared = discover.clone();
        self.discover = Box::new(move |app: &App| (shared.lock().as_ref())(app));
        Seed {
            config: self.config.clone(),
            store: self.store.clone(),
            pid: self.pid,
            desktop: self.desktop.clone(),
            screen: self.screen.clone(),
            learned: self.learned.clone(),
            discover,
            desk: self.desk,
            fresh_for: self.fresh_for,
            window_wait: self.window_wait,
        }
    }
}

impl<D: Desktop> Seed<D> {
    /// A guard for one read-only call ([`super::super::policy::Kind::reads_alone`]),
    /// beside the task's own guard.
    pub fn reader(&self) -> Guard<D> {
        let mut reader = Guard::assemble(
            self.config.clone(),
            self.store.clone(),
            self.pid,
            self.desktop.clone(),
            self.screen.clone(),
            self.learned.clone(),
        );
        let discover = self.discover.clone();
        reader.discover = Box::new(move |app: &App| (discover.lock().as_ref())(app));
        reader.desk = self.desk;
        reader.fresh_for = self.fresh_for;
        reader.window_wait = self.window_wait;
        reader.reader = true;
        reader
    }
}
