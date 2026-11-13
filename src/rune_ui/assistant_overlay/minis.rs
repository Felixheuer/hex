//! The person's desk at the orb: the app windows and browsers quick turns
//! opened (`assistant::guard::desk`), each a small live picture with ×,
//! staying after the turn until the person (×) or the assistant
//! ("schließ WhatsApp") lets it go. A window's picture comes from
//! `app_live` (task [`DESK`]), a browser's from `live` (its turn's task).
//! A press on a window brings it to the person; on a browser, opens it
//! large.
//!
//! The desk lives in the guard's files, written by other processes, so a
//! thread looks at it every second and wakes the orb when it changed.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rune::{El, col, el, icon, row, text};

use super::super::controls::Paint;
use super::super::icons::{GLOBE, PANEL};
use super::jobs::{THUMB_H, THUMB_W};
use super::{app_live, live, look, stack};
use crate::assistant::TaskWindow;
use crate::desktop::i18n::{tr, tr_fill};

/// The desk's task in `app_live`.
pub(crate) const DESK: &str = crate::assistant::guard::DESK;
/// How often the desk is read.
const EVERY: Duration = Duration::from_millis(1000);

/// What is on the desk.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Desk {
    pub(crate) windows: Vec<TaskWindow>,
    /// Quick turns whose browsers stay.
    pub(crate) browsers: Vec<String>,
}

impl Desk {
    pub(crate) fn is_empty(&self) -> bool {
        self.windows.is_empty() && self.browsers.is_empty()
    }
}

fn state() -> std::sync::MutexGuard<'static, Desk> {
    static STATE: Mutex<Desk> = Mutex::new(Desk {
        windows: Vec::new(),
        browsers: Vec::new(),
    });
    STATE.lock().unwrap_or_else(|error| error.into_inner())
}

/// The desk as last read.
pub(crate) fn now() -> Desk {
    state().clone()
}

/// Starts reading the desk every second; `changed` runs (on that thread)
/// when it changed.
pub(crate) fn watch(changed: fn()) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() || cfg!(test) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("hex-desk".into())
        .spawn(move || {
            loop {
                let read = Desk {
                    windows: crate::assistant::guard::desk(),
                    browsers: crate::assistant::guard::desk_browsers(),
                };
                let differs = *state() != read;
                if differs {
                    *state() = read;
                    changed();
                }
                std::thread::sleep(EVERY);
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "couldn't start watching the desk");
    }
}

/// The desk's row at the orb; `None` when it's empty.
pub(crate) fn view<S: 'static>(
    p: Paint,
    desk: &Desk,
    still: bool,
    actions: stack::Actions<S>,
) -> Option<El> {
    let desk = desk.clone();
    if desk.is_empty() {
        return None;
    }
    let apps = app_live::previews(DESK);
    let mut cells: Vec<El> = apps
        .iter()
        .map(|preview| app_cell::<S>(p, preview, still))
        .collect();
    for task in &desk.browsers {
        for preview in live::previews(task) {
            cells.push(browser_cell(p, task, &preview, still, actions));
        }
    }
    // Windows whose picture hasn't come yet: their name.
    for window in &desk.windows {
        let number = window
            .window_id
            .as_deref()
            .and_then(crate::assistant::computer::window_number);
        if apps.iter().any(|preview| Some(preview.number) == number) {
            continue;
        }
        cells.push(waiting_cell::<S>(p, window));
    }
    if cells.is_empty() {
        return None;
    }
    Some(
        look::material(
            p,
            col()
                .key("desk-minis")
                .w(super::jobs::CARD_W)
                .padding(11.0)
                .gap(6.0),
        )
        .role("group")
        .label(tr("Your desk"))
        .child(
            text(tr("Your desk"))
                .key("desk-title")
                .font(look::label(p))
                .color(p.muted()),
        )
        .child(
            row()
                .key("desk-cells")
                .gap(6.0)
                .wrap()
                .role("list")
                .children(cells),
        ),
    )
}

/// A window's live picture with its name and ×.
fn app_cell<S: 'static>(p: Paint, preview: &app_live::AppPreview, still: bool) -> El {
    let picture = super::jobs::mini(
        p,
        format!("desk-app-{}", preview.number),
        &preview.pixels,
        look::liveness(preview.clock.age(preview.has_frame()), preview.failed()),
        (&PANEL, &preview.window.app),
        still,
    );
    let number = preview.number;
    let name = preview.window.app.clone();
    let id = preview.window.window_id.clone().unwrap_or_default();
    let picture = p.k.interactive(
        picture
            .role("button")
            .label(tr_fill("Show {}", &name))
            .hit()
            .on_press(move |_: &mut S| app_live::show(DESK, number)),
    );
    cell::<S>(
        p,
        format!("desk-app-cell-{number}"),
        picture,
        &name,
        move || {
            crate::assistant::guard::desk_release(&id);
        },
    )
}

/// A browser's live picture with its page's host and ×.
fn browser_cell<S: 'static>(
    p: Paint,
    task: &str,
    preview: &live::Preview,
    still: bool,
    actions: stack::Actions<S>,
) -> El {
    let page = preview.page();
    let title = if page.title.is_empty() {
        live::host_of(&page.url).unwrap_or_else(|| tr("Browser").to_owned())
    } else {
        page.title.clone()
    };
    let picture = super::jobs::mini(
        p,
        format!("desk-browser-{}", preview.session),
        &preview.pixels,
        look::liveness(
            preview.clock.age(preview.has_frame()),
            page.failed.is_some(),
        ),
        (&GLOBE, &title),
        still,
    );
    let name = live::host_of(&page.url).unwrap_or_else(|| tr("Browser").to_owned());
    let open = actions.open_preview;
    let (owner, session) = (task.to_owned(), preview.session.clone());
    let picture = p.k.interactive(
        picture
            .role("button")
            .label(tr_fill("Open {} live", &name))
            .hit()
            .on_press(move |s: &mut S| open(s, &owner, &session)),
    );
    let task = task.to_owned();
    cell::<S>(
        p,
        format!("desk-browser-cell-{}", preview.session),
        picture,
        &name,
        move || {
            let task = task.clone();
            let _ = std::thread::Builder::new()
                .name("hex-desk-close".into())
                .spawn(move || crate::assistant::browser::close_now(&task));
        },
    )
}

/// A window on the desk whose picture hasn't come (or can't: no Screen
/// Recording).
fn waiting_cell<S: 'static>(p: Paint, window: &TaskWindow) -> El {
    let id = window.window_id.clone().unwrap_or_default();
    let picture = el().key(format!("desk-wait-{id}")).child(look::no_picture(
        p,
        &PANEL,
        &window.app,
        look::Liveness::Waiting,
        (THUMB_W, THUMB_H),
    ));
    cell::<S>(
        p,
        format!("desk-wait-cell-{id}"),
        picture,
        &window.app,
        move || {
            crate::assistant::guard::desk_release(&id);
        },
    )
}

/// A picture with its name below and × at its corner.
fn cell<S: 'static>(
    p: Paint,
    key: String,
    picture: El,
    name: &str,
    release: impl Fn() + Clone + 'static,
) -> El {
    let c = &p.k.c;
    let close = p.k.interactive(
        el().key("desk-x")
            .absolute(THUMB_W - 16.0, -4.0)
            .size(18.0, 18.0)
            .radius(9.0)
            .fill(c.bg_base)
            .border(p.hair, c.border_base)
            .center()
            .role("button")
            .label(tr_fill("Let go of {}", name))
            .hit()
            .on_press(move |_: &mut S| release())
            .child(icon(p.k.glyph.close, 10.0).color(p.muted()).a11y_hidden()),
    );
    col()
        .key(key)
        .w(THUMB_W)
        .gap(2.0)
        .child(el().size(THUMB_W, THUMB_H).child(picture).child(close))
        .child(
            text(name.to_owned())
                .truncate()
                .font(look::meta(p))
                .color(p.muted())
                .min_w(0.0),
        )
}
