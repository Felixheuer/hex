//! A task's live pictures as one source, whatever makes them: its hidden
//! browsers' pages ([`super::live`], streamed from agent-browser) and its
//! app windows ([`super::app_live`], captured from the screen). Who shows
//! them (the orb's cards, the wall, the Assistant window, the large card)
//! says so here once, and both sources follow.
//!
//! A mini in the rail wears its job's **ring** ([`ring`]), tier 3 of the
//! three frames (`docs/ux/redesign.md` §3.5): the agent works in this
//! window away from the person's screen. Solid with a glow and a small
//! pointer while the job acts in it now, thin while it's parked and idle,
//! dashed while the person has it; its caption says the same in words
//! ("Safari · in background").

use std::sync::Arc;
use std::time::Instant;

use rune::{Color, El, el, icon};

use super::super::controls::Paint;
use super::app_live::{self, AppPreview};
use super::board::{Phase, Task};
use super::live::{self, Preview, Viewer};
use super::{look, palette};
use crate::assistant::view::ACTING_FOR;
use crate::desktop::i18n::tr;

/// How a mini's ring looks: what its job does in that window now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Ring {
    /// The job acts in it now (or has it in front for a moment).
    Acting,
    /// Parked, the job not acting in it this moment.
    Idle,
    /// The person took the job over: the window is theirs.
    Yours,
}

impl Ring {
    /// `task`'s ring at `now`.
    pub(crate) fn of(task: &Task, now: Instant) -> Ring {
        if task.phase == Phase::Paused {
            return Ring::Yours;
        }
        let acting = task.phase.live()
            && (task.borrowed.is_some() || task.acting_at.is_some_and(|at| now < at + ACTING_FOR));
        if acting { Ring::Acting } else { Ring::Idle }
    }

    /// Its word, after the window's name.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Ring::Acting => tr("acting"),
            Ring::Idle => tr("in background"),
            Ring::Yours => tr("you have it"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Ring::Acting => "acting",
            Ring::Idle => "idle",
            Ring::Yours => "yours",
        }
    }
}

/// A mini's caption: its window's (or page's) `name` and what its job does
/// there ("Safari · in background").
pub(crate) fn caption(name: &str, ring: Ring) -> String {
    format!("{name} · {}", ring.word())
}

/// What a screen reader says for a mini: "Safari, Book Nopa, in
/// background".
pub(crate) fn spoken(name: &str, title: &str, ring: Ring) -> String {
    format!("{name}, {title}, {}", ring.word())
}

/// `picture` (a mini of `size`) in its job's ring of `colour`.
pub(super) fn ring(p: Paint, ring: Ring, colour: Color, picture: El, size: (f32, f32)) -> El {
    const OUT: f32 = 3.0;
    let line = el()
        .key(format!("mini-ring-{}", ring.key()))
        .absolute(-OUT, -OUT)
        .size(size.0 + 2.0 * OUT, size.1 + 2.0 * OUT)
        .radius(8.0 + OUT)
        .a11y_hidden();
    let line = match ring {
        Ring::Acting => line
            .border(2.0, colour)
            .outline(4.0, 0.0, colour.alpha(0.28)),
        Ring::Idle => line.border(1.0, colour.alpha(0.6)),
        Ring::Yours => line.border(1.5, colour).border_dash(6.0, 4.0),
    };
    // Acting: a small pointer in the corner, so it isn't colour alone.
    let pointer = (ring == Ring::Acting).then(|| {
        const BADGE: f32 = 20.0;
        el().key("mini-pointer")
            .absolute(size.0 - BADGE + 4.0, size.1 - BADGE + 4.0)
            .size(BADGE, BADGE)
            .radius(BADGE / 2.0)
            .fill(colour)
            .border(p.hair * 2.0, p.k.c.bg_base)
            .center()
            .a11y_hidden()
            .child(icon(&look::POINTER, 11.0).color(palette::ink(colour, p.k)))
    });
    el().size(size.0, size.1)
        .child(picture)
        .child(line)
        .children(pointer)
}

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
