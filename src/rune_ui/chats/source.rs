//! Where the Chats pane reads conversations and sends requests: the
//! assistant service once the window connects, else the in-memory
//! [`fixture::Store`] (scenes, tests, a window not yet connected).

use super::fixture;
use super::round3_shim::schedules::{self, Repeat, Schedule};
use crate::assistant::{
    Context, Conversation, ConversationId, ConversationSummary, Request, TaskState, TaskStatus,
    TaskWindow,
};

pub(super) enum Source {
    /// Scenes, tests, and a window that hasn't connected yet.
    Fixture(Box<fixture::Store>),
    /// The assistant service.
    Live,
}

impl Source {
    pub(super) fn conversations(&self) -> Vec<ConversationSummary> {
        match self {
            Self::Fixture(store) => store.conversations(),
            Self::Live => crate::assistant::conversations(),
        }
    }

    pub(super) fn conversation(&self, id: &str) -> Option<Conversation> {
        match self {
            Self::Fixture(store) => store.conversation(id),
            Self::Live => crate::assistant::conversation(id),
        }
    }

    pub(super) fn tasks(&self) -> Vec<TaskStatus> {
        match self {
            Self::Fixture(store) => store.tasks.clone(),
            Self::Live => crate::assistant::tasks(),
        }
    }

    pub(super) fn submit(&mut self, request: Request) -> ConversationId {
        match self {
            Self::Fixture(store) => store.submit(request),
            Self::Live => crate::assistant::submit(request),
        }
    }

    pub(super) fn cancel(&mut self, id: &str) {
        match self {
            Self::Fixture(store) => store.cancelled.push(id.to_owned()),
            Self::Live => crate::assistant::cancel(id),
        }
    }

    /// Stops every task (the header's Stop all).
    pub(super) fn stop_all(&mut self, ids: Vec<ConversationId>) {
        match self {
            Self::Fixture(store) => store.cancelled.extend(ids),
            Self::Live => crate::assistant::service::stop_all(),
        }
    }

    pub(super) fn delete(&mut self, id: &str) {
        match self {
            Self::Fixture(store) => store.delete(id),
            Self::Live => crate::assistant::delete(id),
        }
    }

    pub(super) fn set_focus(&mut self, id: Option<ConversationId>) {
        match self {
            Self::Fixture(store) => store.focus = id,
            Self::Live => crate::assistant::set_focus(id),
        }
    }

    /// The person takes `id` over (`paused`) or hands it back.
    pub(super) fn take_over(&mut self, id: &str, paused: bool) {
        match self {
            Self::Fixture(store) => {
                store.taken_over.push((id.to_owned(), paused));
                for task in store.tasks.iter_mut().filter(|task| task.conversation == id) {
                    task.state = if paused {
                        TaskState::Paused
                    } else {
                        TaskState::Running
                    };
                }
            }
            Self::Live if paused => crate::assistant::service::take_over(id),
            Self::Live => crate::assistant::service::hand_back(id),
        }
    }

    /// Continues an interrupted conversation; its id once it runs again.
    pub(super) fn resume(&mut self, id: &str) -> Option<ConversationId> {
        match self {
            Self::Fixture(store) => {
                store.resumed.push(id.to_owned());
                Some(id.to_owned())
            }
            Self::Live => crate::assistant::service::resume_interrupted(id),
        }
    }

    /// The files in `folder` worth a row (sheets, tables, documents), by
    /// name, sorted.
    pub(super) fn files(&self, folder: &str) -> Vec<String> {
        match self {
            Self::Fixture(store) => store.files.get(folder).cloned().unwrap_or_default(),
            Self::Live => {
                let Ok(entries) = std::fs::read_dir(expand_home(folder)) else {
                    return Vec::new();
                };
                let mut names: Vec<String> = entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| super::work::worth_a_row(name))
                    .take(50)
                    .collect();
                names.sort_by_key(|name| name.to_lowercase());
                names
            }
        }
    }

    /// Opens a folder, file, or link; `quick` shows a file in Quick Look.
    pub(super) fn open(&mut self, target: &str, quick: bool) {
        match self {
            Self::Fixture(store) => store.opened.push(target.to_owned()),
            Self::Live => {
                let target = expand_home(target);
                let spawned = if quick {
                    std::process::Command::new("/usr/bin/qlmanage")
                        .arg("-p")
                        .arg(&target)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                } else {
                    std::process::Command::new("/usr/bin/open").arg(&target).spawn()
                };
                if let Err(error) = spawned {
                    tracing::warn!(%error, "could not open a job's result");
                }
            }
        }
    }

    /// The scheduled requests.
    pub(super) fn schedules(&self) -> Vec<Schedule> {
        match self {
            Self::Fixture(store) => store.schedules.clone(),
            Self::Live => schedules::list(),
        }
    }

    pub(super) fn add_schedule(&mut self, text: &str, minute: u32, repeat: Repeat) -> Result<(), String> {
        match self {
            Self::Fixture(store) => {
                if text.is_empty() {
                    return Err("Say what it should do".into());
                }
                store.schedules.push(Schedule {
                    id: format!("schedule-{}", store.schedules.len() + 1),
                    text: text.into(),
                    minute_of_day: minute,
                    repeat,
                    enabled: true,
                    next_ms: None,
                });
                Ok(())
            }
            Self::Live => schedules::add(text, minute, repeat)
                .map(|_| ())
                .map_err(|error| format!("{error:#}")),
        }
    }

    pub(super) fn remove_schedule(&mut self, id: &str) {
        match self {
            Self::Fixture(store) => store.schedules.retain(|schedule| schedule.id != id),
            Self::Live => schedules::remove(id),
        }
    }

    pub(super) fn set_schedule_enabled(&mut self, id: &str, on: bool) {
        match self {
            Self::Fixture(store) => {
                for schedule in store.schedules.iter_mut().filter(|schedule| schedule.id == id) {
                    schedule.enabled = on;
                }
            }
            Self::Live => schedules::set_enabled(id, on),
        }
    }

    /// What schedule `id` asks, to run it now.
    pub(super) fn run_schedule(&self, id: &str) -> Option<String> {
        match self {
            Self::Fixture(store) => store
                .schedules
                .iter()
                .find(|schedule| schedule.id == id)
                .map(|schedule| schedule.text.clone()),
            Self::Live => schedules::run_now(id),
        }
    }

    /// What a Here question typed in Chats is about: the frontmost window
    /// that isn't Hex's own (Chats is in front while it's typed).
    pub(super) fn here_context(&self) -> Context {
        match self {
            Self::Fixture(_) => Context {
                frontmost_app: Some("Safari".into()),
                window_title: Some("Hex — Voice to text".into()),
                window: Some(TaskWindow {
                    app: "Safari".into(),
                    window_id: Some("w-42".into()),
                }),
                ..Context::default()
            },
            Self::Live => live_here_context(),
        }
    }
}

/// `~/…` in the person's home folder.
fn expand_home(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => crate::app_paths::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| path.into()),
        None => path.into(),
    }
}

/// [`crate::assistant::computer::quick_context`], with the window in front
/// of Hex's own as the Here lane's window.
fn live_here_context() -> Context {
    let mut context = crate::assistant::computer::quick_context();
    #[cfg(target_os = "macos")]
    {
        let own = std::process::id() as i32;
        let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
        let front = windows.into_iter().find(|window| {
            window.pid != own
                && window.layer == 0
                && window.alpha > 0.0
                && window.bounds[2] > 1.0
                && window.bounds[3] > 1.0
        });
        if let Some(front) = front {
            let hex_in_front = context
                .frontmost_app
                .as_deref()
                .is_none_or(|app| app == crate::app_paths::app_name() || app == front.owner);
            if hex_in_front {
                // What Accessibility read was Hex's own window.
                context.selected_text = None;
                context.frontmost_app = Some(front.owner.clone());
                context.window_title = (!front.title.is_empty()).then(|| front.title.clone());
            }
            context.window = Some(TaskWindow {
                app: front.owner,
                window_id: Some(format!("w-{}", front.id)),
            });
        }
    }
    context
}
