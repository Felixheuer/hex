//! An app's window brought onto this screen when the app runs but shows
//! none here (`actions::recovery`): shown when hidden, restored when
//! minimized, a new window made, or quit quietly and reopened in the
//! background, each step at most once, then the window on screen is the
//! task's like any other. Hidden and minimized take a moment; a relaunch
//! about two seconds.

use std::time::{Duration, Instant};

use serde_json::{Map, json};

use super::super::child::{Desktop, Window};
use super::super::screen::App;
use super::{Guard, same_app};
use crate::assistant::actions::ax::AxWindow;
use crate::assistant::actions::recovery::{self, Recovery, Traits, Tried};

/// How long a quit app has to end before it is reopened.
const QUIT_WAIT: Duration = Duration::from_millis(2500);

/// The steps recovery takes on this Mac; a stand-in in tests, which never
/// touch the person's apps.
pub trait Recoverer: Send {
    /// Whether the app (`pid`) is hidden, and the windows Accessibility
    /// lists for it.
    fn inspect(&mut self, pid: i32) -> (bool, Vec<AxWindow>);
    /// What the app's bundle says it is.
    fn traits(&mut self, app: &App) -> Traits;
    fn unhide(&mut self, pid: i32) -> bool;
    fn leave_full_screen(&mut self, pid: i32, number: u32) -> bool;
    fn new_window(&mut self, script: &str) -> bool;
    fn relaunch(&mut self, pids: &[i64], app: &App) -> bool;
}

/// This Mac.
pub struct System;

impl Recoverer for System {
    fn inspect(&mut self, pid: i32) -> (bool, Vec<AxWindow>) {
        (
            crate::assistant::actions::ax::hidden(pid),
            crate::assistant::actions::ax::windows(pid),
        )
    }

    fn traits(&mut self, app: &App) -> Traits {
        recovery::traits(&app.path)
    }

    fn unhide(&mut self, pid: i32) -> bool {
        recovery::unhide(pid)
    }

    fn leave_full_screen(&mut self, pid: i32, number: u32) -> bool {
        crate::assistant::actions::ax::leave_full_screen(pid, number)
    }

    fn new_window(&mut self, script: &str) -> bool {
        recovery::new_window(script)
    }

    fn relaunch(&mut self, pids: &[i64], app: &App) -> bool {
        recovery::relaunch(pids, app.bundle_id.as_deref(), &app.path, QUIT_WAIT)
    }
}

/// Does nothing and says nothing is hidden or listed (tests).
#[derive(Default)]
pub struct Inert;

impl Recoverer for Inert {
    fn inspect(&mut self, _pid: i32) -> (bool, Vec<AxWindow>) {
        (false, Vec::new())
    }

    fn traits(&mut self, _app: &App) -> Traits {
        Traits::default()
    }

    fn unhide(&mut self, _pid: i32) -> bool {
        false
    }

    fn leave_full_screen(&mut self, _pid: i32, _number: u32) -> bool {
        false
    }

    fn new_window(&mut self, _script: &str) -> bool {
        false
    }

    fn relaunch(&mut self, _pids: &[i64], _app: &App) -> bool {
        false
    }
}

impl<D: Desktop> Guard<D> {
    /// Brings a window of `app` onto this screen; the last step it took, or
    /// why it couldn't (for the model).
    pub(super) fn bring_here(&mut self, app: &App) -> Result<Tried, String> {
        let started = Instant::now();
        let mut tried: Vec<Tried> = Vec::new();
        let traits = self.recover.traits(app);
        loop {
            let pids = self.screen.app_pids(&app.path);
            let full = self.config.front_window;
            let of_app = |window: &Window| {
                window.pid.is_some_and(|pid| pids.contains(&pid))
                    || same_app(&window.app, &app.name)
            };
            let windows = self.screen.windows();
            let on: Vec<String> = windows
                .iter()
                .filter(|window| window.visible && of_app(window) && (full || !window.focused))
                .map(|window| window.id.clone())
                .collect();
            let off_windows: Vec<Window> = self
                .screen
                .off_screen_windows()
                .into_iter()
                .filter(|window| of_app(window) && !on.contains(&window.id))
                .collect();
            let off: Vec<String> = off_windows.iter().map(|window| window.id.clone()).collect();
            let pid = pids
                .first()
                .copied()
                .or_else(|| off_windows.iter().find_map(|window| window.pid))
                .and_then(|pid| i32::try_from(pid).ok());
            let (hidden, ax) = pid.map(|pid| self.recover.inspect(pid)).unwrap_or_default();
            let state = recovery::app_state(
                &app.name,
                app.bundle_id.as_deref(),
                traits,
                hidden,
                &on,
                &off,
                &ax,
            );
            let step = recovery::plan(&state, &tried);
            tracing::info!(app = app.name, ?step, "bringing an app's window here");
            let did = match &step {
                Recovery::Ready(_) => {
                    self.looked = None;
                    tracing::info!(
                        app = app.name,
                        took_ms = started.elapsed().as_millis() as u64,
                        "the app's window is on this screen"
                    );
                    return Ok(tried.last().copied().unwrap_or(Tried::Nothing));
                }
                Recovery::Report(why) => {
                    self.looked = None;
                    return Err(why.clone());
                }
                Recovery::Unhide => pid.is_some_and(|pid| self.recover.unhide(pid)),
                Recovery::LeaveFullScreen(id) => {
                    let number = crate::assistant::computer::window_number(id);
                    match (pid, number) {
                        (Some(pid), Some(number)) => {
                            let left = self.recover.leave_full_screen(pid, number);
                            // The animation out of full screen takes a moment.
                            std::thread::sleep(Duration::from_millis(if cfg!(test) {
                                0
                            } else {
                                900
                            }));
                            left
                        }
                        _ => false,
                    }
                }
                Recovery::Restore(id) => {
                    let name = off_windows
                        .iter()
                        .find(|window| &window.id == id)
                        .map_or_else(|| app.name.clone(), |window| window.app.clone());
                    let mut args = Map::new();
                    args.insert("window_id".into(), json!(id));
                    args.insert("app".into(), json!(name));
                    self.forward("desktop_restore", &args).is_ok()
                }
                Recovery::NewWindow(script) => self.recover.new_window(script),
                Recovery::Relaunch => self.recover.relaunch(&pids, app),
            };
            tried.push(step.kind());
            if did {
                self.wait_on_screen(app);
            }
        }
    }

    /// Whether `app` shows a window in full screen now (a desktop of its
    /// own, which Accessibility can't read from the person's).
    pub(super) fn full_screen(&mut self, app: &App) -> bool {
        self.screen
            .app_pids(&app.path)
            .iter()
            .filter_map(|pid| i32::try_from(*pid).ok())
            .any(|pid| {
                self.recover
                    .inspect(pid)
                    .1
                    .iter()
                    .any(|window| window.full_screen)
            })
    }

    /// Waits (at most `window_wait`) for a window of `app` on this screen.
    fn wait_on_screen(&mut self, app: &App) {
        let waited = Instant::now();
        while waited.elapsed() < self.window_wait {
            let pids = self.screen.app_pids(&app.path);
            let here = self.screen.windows().iter().any(|window| {
                window.visible
                    && !window.focused
                    && (window.pid.is_some_and(|pid| pids.contains(&pid))
                        || same_app(&window.app, &app.name))
            });
            if here {
                return;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    }
}
