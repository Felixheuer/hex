//! One assistant, one stream: every request, answer, and job in one
//! timeline, oldest at the top and the newest by the composer. Quick
//! answers read as a conversation; jobs are live cards with their state,
//! plan, browsers, question, and results, their steps folded. A click on an
//! entry points the composer at it (reply, or steer while it runs); else the
//! composer asks the assistant anew, which knows the whole stream.

use rune::{El, Tx, col, el, icon, row, text};

use super::super::controls::{Paint, pulsing_dot};
use super::super::list_detail::{self as ld};
use super::super::lists::{Tone, dismissible_error_panel, small_button, tone_tag};
use super::list::{self, INTERRUPTED_ICON, origin_icon};
use super::transcript::{self, READING_W, centred};
use super::{ChatsPane, Hold, Turn, Ui, composer, follow, strip, work};
use crate::assistant::{Conversation, Origin, Outcome, Role};
use crate::desktop::i18n::tr;

/// How many conversations the stream reads (the newest; older ones stay in
/// the store, and the assistant can still read them).
pub(super) const LENGTH: usize = 40;

static FAILED_ICON: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_X;
static THUMB_W: f32 = 168.0;
static THUMB_H: f32 = 105.0;

/// An entry's key, to reveal it.
pub(super) fn entry_key(id: &str) -> String {
    format!("entry-{id}")
}

fn title_of(pane: &ChatsPane, id: &str) -> String {
    title(pane, id)
}

/// What an entry is called: its stored title, its task's, or its question.
pub(super) fn title(pane: &ChatsPane, id: &str) -> String {
    pane.stream
        .iter()
        .find(|conversation| conversation.id == id)
        .map(|conversation| conversation.title.clone())
        .or_else(|| pane.task(id).map(|task| task.title.clone()))
        .or_else(|| pane.turns.get(id).map(|turn| turn.question.clone()))
        .or_else(|| {
            pane.list
                .iter()
                .find(|row| row.id == id)
                .map(|row| row.title.clone())
        })
        .unwrap_or_default()
}

/// The pane's body: notices, the running tasks, the stream, the composer.
pub(super) fn body(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let pane = &ui.chats;
    let ready = crate::assistant::ready(&ui.settings);
    let mut notices = Vec::new();
    if let Some(error) = &pane.error {
        notices.push(dismissible_error_panel(
            p,
            "chats-error",
            tr("Couldn't send"),
            error,
            |ui: &mut Ui| ui.chats.error = None,
        ));
    }
    let content = if !ready {
        transcript::finish_setup(ui, p)
    } else if pane.stream.is_empty() && pane.turns.is_empty() && pane.failures.is_empty() {
        col()
            .key("stream-empty")
            .grow(1.0)
            .shrink()
            .min_h(0.0)
            .child(transcript::new_chat(ui, p))
    } else {
        timeline(ui, p)
    };
    col()
        .key("chats-pane")
        .grow(1.0)
        .shrink()
        .children((!notices.is_empty()).then(|| ld::notices(notices)))
        .children(strip::task_strip(ui, p))
        .child(
            col()
                .key("stream")
                .label(tr("Assistant"))
                .grow(1.0)
                .shrink()
                .min_h(0.0)
                .mr(ld::HANG)
                .child(content)
                .children(ready.then(|| composer::composer(ui, p, layer))),
        )
}

/// One entry in the stream: stored, in flight, or both.
enum Entry<'a> {
    Stored(&'a Conversation),
    /// Not stored (yet): its first request in flight, or failed.
    New(&'a str, Option<&'a Turn>),
}

impl Entry<'_> {
    fn id(&self) -> &str {
        match self {
            Entry::Stored(conversation) => &conversation.id,
            Entry::New(id, _) => id,
        }
    }
}

/// The schedules, then every entry by when it was asked, a day's heading
/// where the day changes, kept to its end while new things arrive.
fn timeline(ui: &Ui, p: Paint) -> El {
    let pane = &ui.chats;
    let mut entries: Vec<Entry> = pane.stream.iter().map(Entry::Stored).collect();
    let stored = |id: &str| pane.stream.iter().any(|stored| stored.id == id);
    let mut new: Vec<(&String, &Turn)> = pane.turns.iter().filter(|(id, _)| !stored(id)).collect();
    new.sort_by(|a, b| (a.1.started_ms, a.0).cmp(&(b.1.started_ms, b.0)));
    entries.extend(new.into_iter().map(|(id, turn)| Entry::New(id, Some(turn))));
    let mut failed: Vec<&String> = pane
        .failures
        .keys()
        .filter(|id| !stored(id) && !pane.turns.contains_key(*id))
        .collect();
    failed.sort();
    entries.extend(failed.into_iter().map(|id| Entry::New(id, None)));
    let now = crate::history::now_ms();
    let mut column = col()
        .key("timeline")
        .role("list")
        .label(tr("Assistant"))
        .w_full()
        .max_w(READING_W)
        .gap(18.0)
        .pt(8.0)
        .pb(16.0)
        .child(super::schedules::group(ui, p));
    let mut heading: Option<&str> = None;
    for (index, entry) in entries.iter().enumerate() {
        let at = match entry {
            Entry::Stored(conversation) => conversation.created_ms,
            Entry::New(_, Some(turn)) => turn.started_ms,
            Entry::New(_, None) => now,
        };
        let label = list::group(at, now);
        if heading != Some(label) {
            column = column.child(day(p, index, label));
            heading = Some(label);
        }
        column = column.child(entry_el(ui, p, entry, now));
    }
    let scroll = ld::card_scroll_sticky(p, follow::SCROLL, centred("stream-column", column), true)
        .on_scroll_offset(follow::on_scroll(pane.follow.scroll.clone()));
    col()
        .key("scroll-stack")
        .grow(1.0)
        .shrink()
        .min_h(0.0)
        .child(scroll)
        .children(composer::pills(ui, p))
}

/// A day's heading between entries.
fn day(p: Paint, index: usize, label: &str) -> El {
    row()
        .key(format!("day{index}"))
        .role("heading")
        .level(3)
        .justify_center()
        .pt(4.0)
        .child(
            text(label.to_owned())
                .key("label")
                .font(p.k.ty.small)
                .color(p.faint()),
        )
}

/// How an entry stands, for its head: words and their tone.
fn standing(ui: &Ui, p: Paint, id: &str, outcome: Option<Outcome>) -> Option<El> {
    let pane = &ui.chats;
    if pane.running(id) {
        return Some(match pane.task(id).and_then(|task| pane.hold(task)) {
            Some(Hold::Asking(_)) => tone_tag(p, "state", tr("Needs your answer"), Tone::Accent),
            Some(Hold::Paused) => tone_tag(p, "state", tr("You have it"), Tone::Warning),
            Some(Hold::Waiting { .. }) => tone_tag(p, "state", tr("Waiting"), Tone::Neutral),
            None => tone_tag(p, "state", tr("Running"), Tone::Accent),
        });
    }
    if pane.failures.contains_key(id) || outcome == Some(Outcome::Failed) {
        return Some(tone_tag(p, "state", tr("Failed"), Tone::Danger));
    }
    match outcome {
        Some(Outcome::Interrupted) => Some(tone_tag(
            p,
            "state",
            tr("Interrupted: Hex quit while it ran"),
            Tone::Warning,
        )),
        Some(Outcome::Cancelled) => Some(tone_tag(p, "state", tr("Stopped"), Tone::Neutral)),
        _ => None,
    }
}

/// An entry's leading mark: its task's while it runs, else how it ended
/// when that went wrong, else how it was asked.
fn mark(ui: &Ui, p: Paint, id: &str, origin: Origin, outcome: Option<Outcome>) -> El {
    let pane = &ui.chats;
    let boxed = |child: El| el().key("mark").size(14.0, 14.0).center().child(child);
    if let Some(task) = pane.task(id) {
        return boxed(strip::mark(ui, p, task));
    }
    if pane.running(id) {
        return boxed(pulsing_dot(p.accent()));
    }
    if pane.failures.contains_key(id) || outcome == Some(Outcome::Failed) {
        return boxed(icon(FAILED_ICON, 14.0).color(p.danger()));
    }
    if outcome == Some(Outcome::Interrupted) {
        return boxed(icon(INTERRUPTED_ICON, 14.0).color(p.warning()));
    }
    boxed(icon(origin_icon(origin), 14.0).color(p.faint()))
}

/// A job: asked in the background, moved there, or with a plan.
fn is_job(ui: &Ui, id: &str, origin: Origin) -> bool {
    origin == Origin::Background
        || work::escalated(ui, &id.to_owned())
        || ui.chats.plans.contains_key(id)
}

/// One entry: its head (a click points the composer at it), then what was
/// asked and answered; a job's in a card with its plan, browsers, and
/// results.
fn entry_el(ui: &Ui, p: Paint, entry: &Entry, now: u64) -> El {
    let pane = &ui.chats;
    let id = entry.id();
    let turn = pane.turns.get(id);
    let summary = pane.list.iter().find(|row| row.id == id);
    let (title, origin, updated) = match entry {
        Entry::Stored(conversation) => (
            conversation.title.clone(),
            turn.map_or(conversation.origin, |turn| turn.origin),
            Some(conversation.updated_ms),
        ),
        Entry::New(_, turn) => (
            title_of(pane, id),
            turn.map_or(Origin::Chat, |turn| turn.origin),
            None,
        ),
    };
    let outcome = summary.map(|row| row.outcome);
    let job = is_job(ui, id, origin);
    let running = pane.running(id);
    let targeted = pane.target.as_deref() == Some(id);
    let selected = pane.selected.as_deref() == Some(id);

    let mut meta = Vec::new();
    if job && running && work::escalated(ui, &id.to_owned()) {
        meta.push(tone_tag(
            p,
            "escalated",
            tr("Moved to the background"),
            Tone::Accent,
        ));
    }
    meta.extend(standing(ui, p, id, outcome));
    if !running && let Some(updated) = updated {
        meta.push(ld::meta_text(p, &super::age(updated, now)).key("age"));
    }
    let pick = id.to_owned();
    let head = row()
        .key(format!("chat-{id}"))
        .role("button")
        .label(title.clone())
        .help(tr(
            "Click to reply to this, or to tell it something while it works",
        ))
        .gap(8.0)
        .center_y()
        .py(2.0)
        .child(mark(ui, p, id, origin, outcome))
        .child(
            text(title)
                .key("title")
                .font(if job { p.k.ty.ui_medium } else { p.k.ty.small })
                .color(if job { p.text() } else { p.muted() })
                .grow(1.0)
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
        .children(meta)
        .children(actions(pane, p, id, selected))
        .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
            tx.instant();
            ui.chats.pick(pick.clone());
            tx.focus("composer-input", false);
        });

    let mut body = col().key("body").gap(14.0);
    if let Some(plan) = work::plan_card(ui, p, id) {
        body = body.child(plan);
    }
    let messages = match entry {
        Entry::Stored(conversation) => &conversation.messages[..],
        Entry::New(..) => &[][..],
    };
    for (index, message) in messages.iter().enumerate() {
        body = body.child(match message.role {
            Role::User => transcript::question(ui, p, id, index, message),
            Role::Assistant => transcript::answer(ui, p, id, index, message),
        });
    }
    if running {
        body = body.children(browsers(p, id));
        if turn.is_none() {
            // A task started before this window opened: its question
            // still takes the answer here.
            body = body.children(work::question_block(ui, p, id));
        }
    }
    if let Some(turn) = turn {
        let asked = messages
            .last()
            .is_some_and(|last| last.role == Role::User && last.text().trim() == turn.question);
        if !asked {
            body = body.child(transcript::bubble(p, "pending-question", &turn.question));
        }
        body = body.child(transcript::in_flight(ui, p, id, turn));
    } else {
        if let Entry::Stored(conversation) = entry
            && let Some(results) = work::results(ui, p, conversation)
        {
            body = body.child(results);
        }
        if let Some(failure) = pane.failures.get(id)
            && !matches!(entry, Entry::Stored(conversation) if transcript::ends_in_error(conversation))
        {
            body = body.child(transcript::error_row(p, "failure", failure));
        }
    }
    body = body.children(composer::interrupted_banner(ui, p, id));

    let c = &p.k.c;
    let mut card = col()
        .key(entry_key(id))
        .role("listitem")
        .w_full()
        .gap(10.0)
        .child(head)
        .child(body);
    if job {
        card = card
            .px(14.0)
            .py(12.0)
            .radius(p.k.radius.lg)
            .fill(c.layer_01)
            .border(
                p.hair.max(1.0),
                if targeted { p.accent() } else { c.border_weak },
            );
    } else if targeted {
        card = card.pl(10.0).border_sides([0.0, 0.0, 0.0, 2.0], p.accent());
    }
    if selected && !targeted && !job {
        card = card
            .pl(10.0)
            .border_sides([0.0, 0.0, 0.0, 2.0], c.border_weak);
    }
    card.transition(super::super::onboarding_kit::arrives())
}

/// The selected entry's Delete…, then its confirmation.
fn actions(pane: &ChatsPane, p: Paint, id: &str, selected: bool) -> Vec<El> {
    if !selected {
        return Vec::new();
    }
    if pane.delete_armed.as_deref() == Some(id) {
        return vec![
            small_button(p, "cancel-delete", tr("Cancel"), |ui: &mut Ui, _| {
                ui.chats.delete_armed = None;
            }),
            rune_kit::Button::new(tr("Delete"))
                .variant(rune_kit::ButtonVariant::Danger)
                .on_press(|ui: &mut Ui| ui.chats.delete_armed())
                .build(p.k)
                .key("confirm-delete"),
        ];
    }
    let delete = id.to_owned();
    vec![ld::ghost_icon(
        p,
        "delete-chat",
        ld::TRASH_ICON,
        tr("Delete…"),
        move |ui: &mut Ui, _| ui.chats.delete_armed = Some(delete.clone()),
    )]
}

/// A running job's browsers, as the orb's cards stream them: a click opens
/// one large.
fn browsers(p: Paint, id: &str) -> Option<El> {
    let previews = super::super::assistant_overlay::live::previews(id);
    let shown: Vec<_> = previews
        .iter()
        .filter(|preview| preview.has_frame())
        .take(3)
        .collect();
    if shown.is_empty() {
        return None;
    }
    let c = &p.k.c;
    Some(
        row()
            .key("browsers")
            .gap(8.0)
            .children(shown.into_iter().map(|preview| {
                let (task, session) = (id.to_owned(), preview.session.clone());
                el().key(format!("browser-{}", preview.session))
                    .size(THUMB_W, THUMB_H)
                    .radius(6.0)
                    .fill(c.layer_02)
                    .border(p.hair, p.k.alpha(c.text_base, 0.14))
                    .pixels_fit(&preview.pixels, rune::SurfaceFit::Fill)
                    .role("button")
                    .label(tr("Open live"))
                    .on_press(move |ui: &mut Ui| {
                        super::live_view::open(ui, task.clone(), session.clone());
                    })
            })),
    )
}
