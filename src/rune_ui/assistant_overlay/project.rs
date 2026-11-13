//! Projects on the overlay (`assistant::projects`), kept small: a chip at
//! the rail's head ("● Startup", or a quiet "No project" while the rail
//! shows anyway) that a press unfolds into a compact picker (the recent
//! projects, "No project", "New project…" with its name typed in place);
//! a tiny tag on a job's row when it runs in a project other than the
//! active one; and the Here bubble's one-tap offer ("In project
//! Startup?") when a request seems to be about a project while none is
//! active. Nothing here switches on its own.
//!
//! Several projects at once: each project has its own colour ([`hue`],
//! from its name), its dot on every job's card, question, and result; the
//! rail groups running jobs under a header per project ([`header`]) once
//! more than one group runs; and the picker counts each one's jobs.
//!
//! Plain state and drawing; the host does the switching
//! (`host::orb_panel`).

use rune::{El, TextEdit, col, el, icon, row, text};

use super::super::controls::Paint;
use super::{look, palette};
use crate::assistant::projects::Snapshot;
use crate::desktop::i18n::{tr, tr_fill};

/// The picker's width.
pub(crate) const PICKER_W: f32 = 240.0;
/// A picker row's height.
const ROW_H: f32 = 28.0;
/// The chip's height: a little below the orb's, so it reads as its note.
const CHIP_H: f32 = 30.0;
/// A name's most characters on the chip, alone and beside the orb.
const CHIP_CHARS: usize = 22;
const CHIP_CHARS_BESIDE: usize = 12;

/// The chip and its picker.
#[derive(Clone, Debug, Default)]
pub(crate) struct Picker {
    /// The active project and the recent ones, as last read.
    pub(crate) snapshot: Snapshot,
    /// The picker is unfolded.
    pub(crate) open: bool,
    /// "New project…": the name being typed.
    pub(crate) naming: Option<TextEdit>,
}

impl Picker {
    /// Whether the chip shows on its own (no job needs the rail): a
    /// project is active, or the picker is open.
    pub(crate) fn shows(&self) -> bool {
        self.snapshot.active.is_some() || self.open
    }

    /// The active project's name.
    pub(crate) fn active(&self) -> Option<&str> {
        self.snapshot.active.as_ref().map(|(_, name)| name.as_str())
    }

    /// The typed name, trimmed; `None` while empty.
    pub(crate) fn typed(&self) -> Option<String> {
        self.naming
            .as_ref()
            .map(|edit| edit.text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty())
    }
}

/// What the chip and the picker ask of the host.
pub(crate) struct Actions<S> {
    /// The chip: unfold or fold the picker.
    pub(crate) toggle: fn(&mut S),
    /// A project picked (its id), or "No project" (`None`).
    pub(crate) pick: fn(&mut S, Option<&str>),
    /// "New project…": the name field shows.
    pub(crate) name: fn(&mut S),
    pub(crate) edit: fn(&mut S) -> &mut TextEdit,
    /// Return in the field: the project is made and becomes active.
    pub(crate) create: fn(&mut S),
    /// Escape: the field (or the picker) goes.
    pub(crate) cancel: fn(&mut S),
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

impl<S: 'static> Actions<S> {
    /// Presses that do nothing, for drawings.
    pub(crate) fn inert() -> Self {
        Self {
            toggle: |_| {},
            pick: |_, _| {},
            name: |_| {},
            edit: |_| unreachable!("a drawing without a name field"),
            create: |_| {},
            cancel: |_| {},
        }
    }
}

/// `name` cut to `max` characters with "…".
fn cut(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_owned();
    }
    let mut cut: String = name.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

/// A project's own colour, the same everywhere it shows (chip, picker,
/// group header, a job's dot), from its name: one of the task colours.
pub(crate) fn hue(p: Paint, name: &str) -> rune::Color {
    let index = name
        .to_lowercase()
        .bytes()
        .fold(2166136261u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16777619)
        });
    palette::task((index % 8) as u8, p.k.dark)
}

/// The project's dot in its colour (`name`), or a ring for none.
pub(crate) fn mark(p: Paint, name: Option<&str>, size: f32) -> El {
    let base = el().size(size, size).radius(size / 2.0).a11y_hidden();
    match name {
        Some(name) => base.fill(hue(p, name)),
        None => base.border(1.25, p.faint()),
    }
}

/// A group's header in the rail, while jobs of more than one project run:
/// the project's dot and name ("Without a project" for jobs in none) and
/// how many run.
pub(crate) fn header(p: Paint, name: Option<&str>, count: usize) -> El {
    let words = name.map_or_else(|| tr("Without a project").to_owned(), |name| cut(name, 26));
    let pill = row()
        .key(format!("project-group-{}", name.unwrap_or("none")))
        .h(20.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(10.0)
        // Its own quiet material, so it reads over any wallpaper.
        .fill(p.k.c.bg_base.alpha(0.9))
        .border(p.hair, p.k.c.border_base)
        .role("heading")
        .label(words.clone())
        .child(mark(p, name, 7.0))
        .child(
            text(words)
                .font(look::meta(p))
                .color(p.muted())
                .truncate()
                .min_w(0.0)
                .shrink()
                .a11y_hidden(),
        )
        .child(
            text(count.to_string())
                .font(look::meta(p))
                .color(p.faint())
                .a11y_hidden(),
        );
    row().key("project-group").w_full().pt(2.0).child(pill)
}

/// The chip at the rail's head: "● Startup ▾", or "○ No project ▾".
/// `beside` the orb and its other chips it keeps short: the name cut
/// sooner, and no project only "○ ▾" (its words for VoiceOver).
pub(crate) fn chip<S: 'static>(p: Paint, picker: &Picker, beside: bool, actions: Actions<S>) -> El {
    let c = &p.k.c;
    let active = picker.active();
    let most = if beside {
        CHIP_CHARS_BESIDE
    } else {
        CHIP_CHARS
    };
    let words = match active {
        Some(name) => Some(cut(name, most)),
        None if beside => None,
        None => Some(tr("No project").to_owned()),
    };
    let label = match active {
        Some(name) => tr_fill("Project: {}", name),
        None => tr("No project").to_owned(),
    };
    let toggle = actions.toggle;
    let pill = p.k.interactive(
        row()
            .key("project-chip")
            .min_w(CHIP_H + 16.0)
            .shrink()
            .h(CHIP_H)
            .pl(11.0)
            .pr(9.0)
            .gap(6.0)
            .center_y()
            .radius(CHIP_H / 2.0)
            .fill(c.bg_base.alpha(0.94))
            .border(p.hair, c.border_base)
            .box_shadow(rune::Shadow {
                x: 0.0,
                y: 3.0,
                blur: 12.0,
                spread: 0.0,
                color: rune::Color::BLACK.alpha(if p.k.dark { 0.4 } else { 0.12 }),
                inset: false,
            })
            .role("button")
            .label(label)
            .hit()
            .on_press(move |s: &mut S| toggle(s))
            .child(mark(p, active, 7.0))
            .children(words.map(|words| {
                text(words)
                    .key("project-chip-name")
                    .font(look::label(p))
                    .color(if active.is_some() {
                        p.text()
                    } else {
                        p.muted()
                    })
                    .truncate()
                    .min_w(0.0)
                    .shrink()
            }))
            .child(
                icon(&rune_theme_opencode::icons::CHEVRON_DOWN, 12.0)
                    .color(p.faint())
                    .a11y_hidden(),
            ),
    );
    if active.is_some() || picker.open {
        pill
    } else {
        pill.opacity(0.8)
    }
}

/// One row of the picker: a check where it is the active one, its mark,
/// and its words.
fn choice<S: 'static>(
    p: Paint,
    key: String,
    lead: El,
    words: String,
    checked: bool,
    on_press: impl Fn(&mut S) + 'static,
) -> El {
    let check = el()
        .w(14.0)
        .center()
        .children(checked.then(|| icon(&look::CHECK, 13.0).color(p.text()).a11y_hidden()));
    p.k.interactive(
        row()
            .key(key)
            .w_full()
            .h(ROW_H)
            .px(8.0)
            .gap(8.0)
            .center_y()
            .radius(8.0)
            .role("menuitemradio")
            .label(words.clone())
            .hit()
            .on_press(on_press)
            .child(check)
            .child(lead)
            .child(
                text(words)
                    .truncate()
                    .font(row_font(p))
                    .color(p.text())
                    .min_w(0.0)
                    .grow(1.0),
            ),
    )
}

/// The picker under the chip: the recent projects (the active one
/// ticked), "No project", and "New project…", whose name is typed in
/// place (Return makes it and makes it active; Esc cancels).
/// `running`: each project's running jobs, by name, shown beside it.
pub(crate) fn picker<S: 'static>(
    p: Paint,
    picker: &Picker,
    running: &[(String, usize)],
    actions: Actions<S>,
) -> El {
    let active = picker.snapshot.active.as_ref().map(|(id, _)| id.clone());
    let pick = actions.pick;
    let mut rows: Vec<El> = picker
        .snapshot
        .recent
        .iter()
        .map(|(id, name)| {
            let chosen = id.clone();
            let jobs = running
                .iter()
                .find(|(running, _)| running == name)
                .map_or(0, |(_, count)| *count);
            choice(
                p,
                format!("project-{name}"),
                mark(p, Some(name), 7.0),
                cut(name, 30),
                active.as_deref() == Some(id.as_str()),
                move |s: &mut S| pick(s, Some(&chosen)),
            )
            .children((jobs > 0).then(|| count_badge(p, name, jobs)))
        })
        .collect();
    if !rows.is_empty() {
        rows.push(
            el().key("project-rule")
                .w_full()
                .h(p.hair)
                .my(3.0)
                .fill(p.k.c.border_base)
                .a11y_hidden(),
        );
    }
    rows.push(choice(
        p,
        "project-none".to_owned(),
        mark(p, None, 7.0),
        tr("No project").to_owned(),
        active.is_none(),
        move |s: &mut S| pick(s, None),
    ));
    rows.push(match &picker.naming {
        Some(edit) => field(p, edit, actions),
        None => {
            let name = actions.name;
            choice(
                p,
                "project-new".to_owned(),
                icon(&rune_theme_opencode::icons::PLUS_SMALL, 14.0)
                    .color(p.muted())
                    .w(7.0)
                    .a11y_hidden(),
                tr("New project…").to_owned(),
                false,
                move |s: &mut S| name(s),
            )
        }
    });
    rows.push(
        text(tr("Or say “switch to project …”"))
            .key("project-hint")
            .truncate()
            .font(look::meta(p))
            .color(p.faint())
            .px(8.0)
            .pt(4.0),
    );
    look::material(
        p,
        col()
            .key("project-picker")
            .w(PICKER_W)
            .padding(6.0)
            .gap(1.0)
            .role("menu")
            .label(tr("Projects"))
            .hit(),
    )
    .radius(14.0)
    .children(rows)
}

/// How many of a project's jobs run, at its picker row's end: "2".
fn count_badge(p: Paint, name: &str, jobs: usize) -> El {
    let label = if jobs == 1 {
        tr("1 job running").to_owned()
    } else {
        tr_fill("{} jobs running", &jobs.to_string())
    };
    row()
        .key(format!("project-running-{name}"))
        .h(18.0)
        .min_w(18.0)
        .px(6.0)
        .center()
        .radius(9.0)
        .fill(p.k.c.layer_02)
        .role("status")
        .label(label)
        .child(
            text(jobs.to_string())
                .font(look::meta(p))
                .color(p.muted())
                .a11y_hidden(),
        )
}

/// A picker row's words: 13 pt, between the card's body and its labels.
fn row_font(p: Paint) -> rune::TextStyle {
    rune::TextStyle {
        size: 13.0,
        line_height: 18.0,
        ..p.k.ty.ui
    }
}

/// The new project's name, typed in place.
fn field<S: 'static>(p: Paint, edit: &TextEdit, actions: Actions<S>) -> El {
    let k = p.k;
    let ink = k.c.text_base;
    let create = actions.create;
    let cancel = actions.cancel;
    let input = rune::text_input(edit, row_font(p))
        .key("project-name-input")
        .w_full()
        .color(ink)
        .placeholder(tr("Name of the project"), k.c.text_faint)
        .caret(ink, k.selection())
        .label(tr("Name of the project"))
        .on_edit(actions.edit)
        .on_input_key(
            move |s: &mut S, event: &rune::KeyEvent, _tx: &mut rune::Tx| match event.key {
                rune::Key::Enter => {
                    create(s);
                    true
                }
                rune::Key::Escape => {
                    cancel(s);
                    true
                }
                _ => false,
            },
        );
    col()
        .key("project-new-field")
        .w_full()
        .gap(3.0)
        .py(3.0)
        .child(
            el().key("project-name-box")
                .w_full()
                .px(8.0)
                .py(5.0)
                .radius(8.0)
                .fill(k.c.layer_01)
                .border(1.0, palette::agent(k.dark).alpha(0.6))
                .role("textbox")
                .child(input),
        )
        .child(
            text(tr("Return makes it · Esc cancels"))
                .key("project-name-hint")
                .truncate()
                .font(look::meta(p))
                .color(p.faint())
                .px(2.0),
        )
}

/// A job's tiny project tag ("Startup"), on its row when it runs in a
/// project other than the active one.
pub(crate) fn tag(p: Paint, name: &str) -> El {
    row()
        .key("job-project")
        .h(18.0)
        .px(6.0)
        .gap(4.0)
        .center_y()
        .radius(9.0)
        .fill(p.k.c.layer_02)
        .role("status")
        .label(tr_fill("Project: {}", name))
        .child(mark(p, Some(name), 5.0))
        .child(
            text(cut(name, 16))
                .font(look::meta(p))
                .color(p.muted())
                .a11y_hidden(),
        )
}

/// The Here bubble's offer: "In project Startup?", one tap makes it the
/// active project.
pub(crate) fn offer<S: 'static>(p: Paint, id: &str, name: &str, take: fn(&mut S, &str)) -> El {
    let id = id.to_owned();
    look::button(
        p,
        look::Kind::Secondary,
        &tr_fill("In project {}?", &cut(name, 18)),
        move |s: &mut S| take(s, &id),
    )
    .key("project-offer")
}

/// A card's lead with its project: the task's `dot`, then the project's
/// dot beside it (a question's and a result's), so which project asks or
/// answered shows at a glance; alone without one.
pub(crate) fn with_dot(p: Paint, dot: El, project: Option<&str>) -> El {
    match project {
        Some(name) => row()
            .key("card-project")
            .gap(4.0)
            .center_y()
            .label(tr_fill("Project: {}", name))
            .child(dot)
            .child(mark(p, Some(name), 6.0).key("card-project-dot")),
        None => dot,
    }
}
