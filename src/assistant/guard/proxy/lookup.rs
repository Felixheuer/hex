//! Which windows a call reads or operates, and whether they are the task's:
//! a ref's snapshot, an app's windows, the window in front, the windows
//! under a point; blocked apps; another task's or the person's window.

use serde_json::{Map, Value};

use super::super::child::{self, Desktop, Window};
use super::super::policy::{self, Kind, Target};
use super::super::state::LeaseFile;
use super::{
    Guard, Look, Resolved, bundle_ids, describe_task_window, name_window, refusal, same_app,
};
use crate::assistant::types::Lane;

impl<D: Desktop> Guard<D> {
    pub(super) fn resolve(
        &mut self,
        tool: &str,
        kind: Kind,
        target: Target,
        args: &mut Map<String, Value>,
    ) -> Result<Vec<Resolved>, Value> {
        let background_act = kind.acts() && self.config.lane == Lane::Background;
        let mut found = match target {
            Target::Window(id) => {
                let windows = self.list(Look::Check)?;
                let window = windows.iter().find(|window| window.id == id).ok_or_else(|| {
                    refusal(
                        tool,
                        "WINDOW_NOT_FOUND",
                        &format!("There's no window {id} now; desktop_list_windows shows the open ones."),
                    )
                })?;
                vec![Resolved::of(window)]
            }
            Target::Ref(snapshot) => {
                let origin = self
                    .learned
                    .snapshots
                    .lock()
                    .get(&snapshot)
                    .cloned()
                    .ok_or_else(|| {
                        refusal(
                            tool,
                            "UNKNOWN_REF",
                            "Agent-Desk doesn't know which window this ref is in. Take a snapshot of your \
                         window through these tools and use its refs.",
                        )
                    })?;
                let mut resolved = Resolved {
                    id: origin.window.clone(),
                    app: origin.app.clone(),
                    focused: false,
                };
                if let Some(id) = &origin.window
                    && (background_act || kind == Kind::Press)
                {
                    let windows = self.list(Look::Check)?;
                    resolved.focused = windows
                        .iter()
                        .any(|window| &window.id == id && window.focused);
                }
                vec![resolved]
            }
            Target::App(app) => {
                let windows = self.list(Look::Check)?;
                let of_app: Vec<&Window> = windows
                    .iter()
                    .filter(|window| window.visible && same_app(&window.app, &app))
                    .collect();
                if of_app.is_empty() {
                    return Err(refusal(
                        tool,
                        "WINDOW_NOT_FOUND",
                        &format!("{app} has no window on screen that this task could use."),
                    ));
                }
                let takes_id = policy::TAKES_WINDOW_ID.contains(&tool);
                if of_app.len() == 1 || !takes_id {
                    if of_app.len() == 1 && takes_id {
                        name_window(args, &of_app[0].id);
                    }
                    of_app.into_iter().map(Resolved::of).collect()
                } else {
                    let held: Vec<String> = self
                        .held()
                        .into_iter()
                        .map(|lease| lease.window_id)
                        .collect();
                    let mine: Vec<&&Window> = of_app
                        .iter()
                        .filter(|window| held.contains(&window.id))
                        .collect();
                    match mine.as_slice() {
                        [only] => {
                            name_window(args, &only.id);
                            vec![Resolved::of(only)]
                        }
                        [] => vec![Resolved::of(of_app[0])],
                        _ => {
                            return Err(refusal(
                                tool,
                                "AMBIGUOUS_TARGET",
                                &format!(
                                    "{app} has several of your windows; name one with window_id."
                                ),
                            ));
                        }
                    }
                }
            }
            Target::Focused => {
                let windows = self.list(Look::Check)?;
                let front = windows
                    .iter()
                    .find(|window| window.focused && window.visible)
                    .ok_or_else(|| {
                        refusal(
                            tool,
                            "WINDOW_NOT_FOUND",
                            "No window is in front; name one with window_id.",
                        )
                    })?;
                if self.config.lane == Lane::Background && !self.config.front_window {
                    return Err(refusal(
                        tool,
                        "NOT_YOUR_WINDOW",
                        "Without window_id or app this would use the window in front, which is \
                         the person's. Name one of your windows with window_id.",
                    ));
                }
                if policy::TAKES_WINDOW_ID.contains(&tool) {
                    name_window(args, &front.id);
                }
                vec![Resolved::of(front)]
            }
            Target::Point(x, y) => {
                let windows = self.list(Look::Check)?;
                let under: Vec<Resolved> = windows
                    .iter()
                    .filter(|window| window.visible && window.contains(x, y))
                    .map(Resolved::of)
                    .collect();
                if under.is_empty() {
                    return Err(refusal(
                        tool,
                        "NOT_YOUR_WINDOW",
                        "That point isn't in any window; only points inside your windows are allowed.",
                    ));
                }
                under
            }
        };
        if matches!(kind, Kind::Press | Kind::CloseApp) {
            found = self.whole_app(tool, kind, found)?;
        }
        Ok(found)
    }

    /// Keys go to an app's key window and quitting closes every window, so
    /// both need the app's windows, not just the one named.
    pub(super) fn whole_app(
        &mut self,
        tool: &str,
        kind: Kind,
        found: Vec<Resolved>,
    ) -> Result<Vec<Resolved>, Value> {
        let Some(first) = found.first().cloned() else {
            return Ok(found);
        };
        let windows = self.list(Look::Check)?;
        let of_app: Vec<Resolved> = windows
            .iter()
            .filter(|window| {
                (window.visible || window.minimized) && same_app(&window.app, &first.app)
            })
            .map(Resolved::of)
            .collect();
        if kind == Kind::Press && self.config.lane == Lane::Here {
            let on_screen = of_app.iter().filter(|window| window.id.is_some()).count();
            let front = first.id.as_ref().is_some_and(|id| {
                windows
                    .iter()
                    .any(|window| &window.id == id && window.focused)
            });
            if front || on_screen <= 1 {
                return Ok(found);
            }
            return Err(refusal(
                tool,
                "NOT_YOUR_WINDOW",
                "Keys would go to whichever window of the app is in front, and that isn't this \
                 task's window now.",
            ));
        }
        Ok(if of_app.is_empty() { found } else { of_app })
    }

    pub(super) fn check(
        &mut self,
        tool: &str,
        kind: Kind,
        window: &Resolved,
        held: &[LeaseFile],
    ) -> Result<(), Value> {
        if self.blocked(&window.app) {
            return Err(self.blocked_refusal(tool, &window.app));
        }
        // A question about the screen may look at any window ("what am I
        // doing in Citrix?"); acting stays in the window in front, and
        // anything else is handed on as a job.
        if kind == Kind::Observe && self.config.lane == Lane::Here {
            return Ok(());
        }
        let Some(id) = &window.id else {
            // An app's surface without a window (its menu bar, a menu).
            if held.iter().any(|lease| same_app(&lease.app, &window.app)) {
                return Ok(());
            }
            return Err(self.not_held(tool, None, &window.app));
        };
        if !held.iter().any(|lease| &lease.window_id == id) {
            // Full control: any window not another task's becomes this
            // task's as it acts in it.
            let acquired = self.config.front_window.then(|| {
                self.store.acquire(
                    &self.config.task,
                    self.pid,
                    id,
                    &window.app,
                    crate::desktop::events::now_ms(),
                )
            });
            match acquired {
                None | Some(super::super::state::Acquire::Taken(_)) => {
                    return Err(self.not_held(tool, Some(id), &window.app));
                }
                // Taken into the task as it acts: out of the person's way
                // and live on its card, unless it's the one they're in.
                Some(super::super::state::Acquire::Acquired) if !window.focused => {
                    let listed = self
                        .list(Look::Check)
                        .ok()
                        .and_then(|windows| windows.into_iter().find(|on| &on.id == id));
                    match listed {
                        Some(listed) if listed.focused => {}
                        Some(listed) => self.arrived.push(listed),
                        None => self.arrived.push(super::super::child::Window {
                            id: id.clone(),
                            app: window.app.clone(),
                            title: String::new(),
                            pid: None,
                            focused: false,
                            visible: true,
                            minimized: false,
                            bounds: None,
                        }),
                    }
                }
                Some(_) => {}
            }
        }
        if kind.acts()
            && self.config.lane == Lane::Background
            && window.focused
            && !self.config.front_window
        {
            return Err(refusal(
                tool,
                "PERSON_IN_WINDOW",
                "The person is using that window right now (it's in front). Wait with \
                 desktop_wait and try again, or work in another of your windows.",
            ));
        }
        Ok(())
    }

    pub(super) fn not_held(&self, tool: &str, id: Option<&String>, app: &str) -> Value {
        // A desk window another task has borrowed, or a job hasn't claimed.
        if let Some(refused) = id.and_then(|id| self.desk_refusal(tool, id)) {
            return refused;
        }
        let other = id
            .and_then(|id| self.store.holder(id))
            .filter(|lease| lease.task != self.config.task);
        let whose = if other.is_some() {
            "That window belongs to another task"
        } else {
            "That window belongs to the person"
        };
        let window = id.map_or_else(|| app.to_owned(), |id| format!("{id} ({app})"));
        let message = match self.config.lane {
            Lane::Here => format!(
                "{whose}: {window}. This task may act only in the window that was in front when it \
                 was asked ({}).",
                self.config
                    .here_window
                    .as_ref()
                    .map_or("unknown".into(), describe_task_window)
            ),
            Lane::Background => format!(
                "{whose}: {window}. Open your own window with hex_open_app, or claim a free one \
                 with desktop_claim_window."
            ),
        };
        refusal(tool, "NOT_YOUR_WINDOW", &message)
    }

    pub(super) fn blocked_refusal(&self, tool: &str, app: &str) -> Value {
        refusal(
            tool,
            "BLOCKED_APP",
            &format!("{app} is blocked in Agent-Desk's settings: no task may read or operate it."),
        )
    }

    /// Whether `app` (a name or bundle id) is one the person blocked.
    pub(super) fn blocked(&mut self, app: &str) -> bool {
        if self.config.blocked_apps.is_empty() {
            return false;
        }
        let key = policy::app_key(app);
        let blocked: Vec<String> = self
            .config
            .blocked_apps
            .iter()
            .map(|app| policy::app_key(app))
            .collect();
        if blocked.contains(&key) {
            return true;
        }
        if !blocked.iter().any(|entry| entry.contains('.')) {
            return false;
        }
        if self.bundle_ids.is_none() {
            let apps = self
                .desktop
                .call("desktop_list_apps", &Map::new())
                .ok()
                .and_then(|result| child::data(&result))
                .map(|data| bundle_ids(&data))
                .unwrap_or_default();
            self.bundle_ids = Some(apps);
        }
        self.bundle_ids.iter().flatten().any(|(name, bundle)| {
            (name == &key || bundle == &key) && (blocked.contains(name) || blocked.contains(bundle))
        })
    }
}
