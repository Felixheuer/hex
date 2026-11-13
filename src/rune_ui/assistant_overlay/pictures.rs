//! A task's live pictures as one source, whatever makes them: its hidden
//! browsers' pages ([`super::live`], streamed from agent-browser) and its
//! app windows ([`super::app_live`], captured from the screen). Who shows
//! them (the orb's cards, the wall, the Assistant window, the large card)
//! says so here once, and both sources follow.

use std::sync::Arc;

use super::app_live::{self, AppPreview};
use super::live::{self, Preview, Viewer};

/// One live picture.
pub(crate) enum Picture {
    Browser(Arc<Preview>),
    App(Arc<AppPreview>),
}

impl Picture {
    /// A frame came in.
    pub(crate) fn has_frame(&self) -> bool {
        match self {
            Self::Browser(preview) => preview.has_frame(),
            Self::App(preview) => preview.has_frame(),
        }
    }

    /// Its latest small frame.
    pub(crate) fn pixels(&self) -> &rune::Pixels {
        match self {
            Self::Browser(preview) => &preview.pixels,
            Self::App(preview) => &preview.pixels,
        }
    }
}

/// `task`'s pictures: its app windows, then its browsers.
pub(crate) fn of(task: &str) -> Vec<Picture> {
    let mut pictures: Vec<Picture> = app_live::previews(task)
        .into_iter()
        .map(Picture::App)
        .collect();
    pictures.extend(live::previews(task).into_iter().map(Picture::Browser));
    pictures
}

/// How many pictures `task` has.
pub(crate) fn count(task: &str) -> usize {
    app_live::previews(task).len() + live::previews(task).len()
}

/// The jobs whose cards show at the orb now.
pub(crate) fn cards(tasks: &[String]) {
    live::cards(tasks);
    app_live::cards(tasks);
}

/// The jobs the live wall shows (empty: it's closed).
pub(crate) fn wall(tasks: &[String]) {
    live::wall(tasks);
    app_live::wall(tasks);
}

/// The running jobs the Assistant window shows (empty: it's hidden or on
/// another pane).
pub(crate) fn window(tasks: &[String]) {
    live::window(tasks);
    app_live::window(tasks);
}

/// The overlay's large card shows `task`'s picture `session` (`None`:
/// nothing): only that one streams large.
pub(crate) fn view(target: Option<(&str, &str)>) {
    match target.map(|(task, session)| (task, session, app_live::number_of(session))) {
        Some((task, _, Some(number))) => {
            live::view(Viewer::Overlay, None);
            app_live::view(Some((task, number)));
        }
        Some((task, session, None)) => {
            app_live::view(None);
            live::view(Viewer::Overlay, Some((task, session)));
        }
        None => {
            live::view(Viewer::Overlay, None);
            app_live::view(None);
        }
    }
}
