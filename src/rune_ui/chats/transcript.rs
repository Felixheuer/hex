//! The selected conversation in the detail card: its header, the
//! transcript (questions on the right, answers as Markdown, what the
//! assistant did as compact muted steps, screenshots, errors), the request
//! in flight with its answer streaming and Stop, and the composer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use rune::{El, Tx, Waker, col, el, icon, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Markdown, Spinner, TextArea};

use super::super::controls::{DESCRIPTION, Paint};
use super::super::list_detail::{self as ld, FOLDED, UNFOLDED};
use super::super::lists::{Tone, small_button, tone_tag};
use super::list::{AGENT_ICON, origin_label};
use super::{ChatsPane, Phase, Turn, Ui};
use crate::assistant::{Conversation, Message, Part, Role};
use crate::desktop::i18n::{tr, tr_fill};

static SEND_ICON: &rune::Icon = &rune_theme_opencode::icons::ARROW_UP;
static STOP_ICON: &rune::Icon = &rune_theme_opencode::icons::STOP;
static ERROR_ICON: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_X;

/// The widest a question's bubble grows.
const BUBBLE_MAX: f32 = 380.0;
/// A screenshot thumbnail's width.
const THUMB_W: f32 = 220.0;
/// Step blocks longer than this fold to their last step.
const FOLD_AFTER: usize = 4;

/// Screenshot thumbnails, decoded off the window's thread.
#[derive(Default)]
pub(in crate::rune_ui) struct Thumbnails {
    cache: Option<rune::ImageCache>,
    images: RefCell<HashMap<PathBuf, rune::Image>>,
    /// Scenes: one drawn picture for every screenshot.
    fixture: Option<rune::Image>,
}

impl Thumbnails {
    pub(super) fn connect(&mut self, waker: Waker) {
        self.cache = Some(rune::ImageCache::new(waker));
    }

    /// Every screenshot draws a made-up window, for scenes.
    pub(super) fn fixture(&mut self) {
        let cache = rune::ImageCache::new(std::sync::Arc::new(|| {}));
        let (w, h) = (320_u32, 200_u32);
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let window = (40..280).contains(&x) && (30..176).contains(&y);
                let bar = window && y < 48;
                let [r, g, b] = if bar {
                    [228, 228, 232]
                } else if window {
                    [250, 250, 252]
                } else {
                    [40 + (x / 8) as u8, 90 + (y / 6) as u8, 170]
                };
                pixels.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let image = cache.rgba(w, h, pixels);
        let _ = image.wait();
        while image.pixels().size() == (0, 0) {
            std::thread::yield_now();
        }
        self.cache = Some(cache);
        self.fixture = Some(image);
    }

    /// The thumbnail of `path`, asked for the first time it shows.
    fn get(&self, path: Option<&PathBuf>) -> Option<rune::Image> {
        if let Some(image) = &self.fixture {
            return Some(image.clone());
        }
        let path = path?;
        let cache = self.cache.as_ref()?;
        let mut images = self.images.borrow_mut();
        if images.len() > 64 {
            images.clear();
        }
        Some(
            images
                .entry(path.clone())
                .or_insert_with(|| cache.file(path, THUMB_W, 2.0))
                .clone(),
        )
    }
}

/// The detail card: a new chat, or the selected conversation.
pub(super) fn detail(ui: &Ui, p: Paint) -> El {
    let pane = &ui.chats;
    let content = match pane.selected.as_deref() {
        None => new_chat(ui, p),
        Some(id) => conversation(ui, p, id),
    };
    col()
        .key("chat-detail")
        .grow(1.0)
        .shrink()
        .min_h(0.0)
        .child(content)
        .child(composer(ui, p))
}

/// Nothing selected: what the assistant is for, over the composer.
fn new_chat(ui: &Ui, p: Paint) -> El {
    use super::super::onboarding_kit::{LEAD, keys, title_font};
    let hint = match ui.settings.assistant.action_hotkey.as_ref() {
        Some(binding) => {
            let caps = super::super::settings::shortcut_caps(binding, false);
            row()
                .key("hint")
                .gap(8.0)
                .center_y()
                .child(text(tr("Type below, or hold")).key("type").font(LEAD).color(p.muted()))
                .child(keys(p, "keys", &caps, false))
                .child(text(tr("anywhere")).key("anywhere").font(LEAD).color(p.muted()))
        }
        None => text(tr("Type below to ask the assistant"))
            .key("hint")
            .font(LEAD)
            .color(p.muted()),
    };
    col()
        .key("new-chat-empty")
        .grow(1.0)
        .shrink()
        .center()
        .gap(10.0)
        .pb(40.0)
        .child(
            text(tr("What can I help with?"))
                .key("title")
                .role("heading")
                .level(2)
                .font(title_font(p))
                .color(p.text()),
        )
        .child(hint)
        .transition(super::super::onboarding_kit::arrives())
}

/// The selected conversation: its header and transcript.
fn conversation(ui: &Ui, p: Paint, id: &str) -> El {
    let pane = &ui.chats;
    let stored = pane.shown.as_ref().filter(|shown| shown.id == id);
    let turn = pane.turns.get(id);
    let summary = pane.list.iter().find(|row| row.id == id);
    let title = stored
        .map(|conversation| conversation.title.clone())
        .or_else(|| summary.map(|row| row.title.clone()))
        .or_else(|| turn.map(|turn| turn.question.clone()))
        .unwrap_or_default();
    let origin = stored
        .map(|conversation| conversation.origin)
        .or_else(|| summary.map(|row| row.origin))
        .or_else(|| turn.map(|turn| turn.origin));
    let running = pane.running(id);
    let mut meta = Vec::new();
    if let Some(origin) = origin {
        meta.push(ld::meta_text(p, origin_label(origin)));
    }
    if let Some(updated) = stored.map(|conversation| conversation.updated_ms) {
        meta.push(ld::meta_text(
            p,
            &super::age(updated, crate::history::now_ms()),
        ));
    }
    if running {
        meta.push(tone_tag(p, "running", tr("Running"), Tone::Accent));
    }
    let header = ld::card_header(ld::card_title(p, &title), meta, actions(pane, p, id));
    let mut column = col().key("transcript").gap(18.0).pb(8.0);
    let messages = stored.map_or(&[][..], |conversation| &conversation.messages[..]);
    for (index, message) in messages.iter().enumerate() {
        column = column.child(match message.role {
            Role::User => question(ui, p, id, index, message),
            Role::Assistant => answer(ui, p, id, index, message),
        });
    }
    if let Some(turn) = turn {
        let asked = messages
            .last()
            .is_some_and(|last| last.role == Role::User && last.text().trim() == turn.question);
        if !asked {
            column = column.child(bubble(p, "pending-question", &turn.question));
        }
        column = column.child(in_flight(pane, p, id, turn));
    } else if let Some(failure) = pane.failures.get(id)
        && !stored.is_some_and(ends_in_error)
    {
        column = column.child(error_row(p, "failure", failure));
    }
    col()
        .key("chat")
        .grow(1.0)
        .shrink()
        .min_h(0.0)
        .child(header.pb(4.0))
        .child(ld::card_scroll_sticky(p, "chat-scroll", column, true))
}

fn ends_in_error(conversation: &Conversation) -> bool {
    conversation
        .messages
        .last()
        .and_then(|message| message.parts.last())
        .is_some_and(|part| matches!(part, Part::Error { .. }))
}

/// Stop while it runs; Delete…, then its confirmation.
fn actions(pane: &ChatsPane, p: Paint, id: &str) -> Vec<El> {
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
    let mut actions = Vec::new();
    let delete = id.to_owned();
    actions.push(ld::ghost_icon(
        p,
        "delete-chat",
        ld::TRASH_ICON,
        tr("Delete chat…"),
        move |ui: &mut Ui, _| ui.chats.delete_armed = Some(delete.clone()),
    ));
    actions
}

/// A question: its words in a bubble on the right, screenshots under it.
fn question(ui: &Ui, p: Paint, id: &str, index: usize, message: &Message) -> El {
    let words = message.text();
    let mut block = col()
        .key(format!("message{index}"))
        .gap(8.0)
        .items_end();
    if !words.trim().is_empty() {
        block = block.child(bubble(p, "bubble", &words));
    }
    for (part_index, part) in message.parts.iter().enumerate() {
        if let Part::Screenshot { file } = part {
            block = block.child(screenshot(ui, p, id, part_index, file));
        }
    }
    block
}

/// A question's words in a bubble on the right.
fn bubble(p: Paint, key: &str, words: &str) -> El {
    row().key(key.to_owned()).justify_end().child(
        text(words.to_owned())
            .key("words")
            .font(p.k.ty.body)
            .color(p.text())
            .selectable(true)
            .wrap()
            .max_w(BUBBLE_MAX)
            .px(12.0)
            .py(8.0)
            .radius(p.k.radius.lg)
            .fill(p.k.c.layer_02),
    )
}

/// An answer: its parts in order, a run of actions as one step block.
fn answer(ui: &Ui, p: Paint, id: &str, index: usize, message: &Message) -> El {
    let mut block = col().key(format!("message{index}")).gap(10.0);
    let mut steps: Vec<&str> = Vec::new();
    let flush = |block: El, steps: &mut Vec<&str>, at: usize| {
        if steps.is_empty() {
            return block;
        }
        let key = format!("{id}/{index}/{at}");
        let folded = !ui.chats.unfolded.contains(&key);
        let list = step_block(p, &key, steps, folded);
        steps.clear();
        block.child(list)
    };
    for (part_index, part) in message.parts.iter().enumerate() {
        if let Part::Action { description } = part {
            steps.push(description);
            continue;
        }
        block = flush(block, &mut steps, part_index);
        block = block.child(match part {
            Part::Text { text } => markdown(p, &format!("text{part_index}"), text, false),
            Part::Screenshot { file } => screenshot(ui, p, id, part_index, file).self_start(),
            Part::Error { message } => error_row(p, &format!("error{part_index}"), message),
            Part::Action { .. } => unreachable!("actions gather into step blocks"),
        });
    }
    flush(block, &mut steps, message.parts.len())
}

fn markdown(p: Paint, key: &str, source: &str, streaming: bool) -> El {
    Markdown::new(source.to_owned())
        .streaming(streaming)
        .selectable(true)
        .build(p.k)
        .key(key.to_owned())
}

/// What the assistant did, as compact muted rows. A long run folds to its
/// last step under a "N steps" toggle.
fn step_block(p: Paint, key: &str, steps: &[&str], folded: bool) -> El {
    let foldable = steps.len() > FOLD_AFTER;
    let shown: &[&str] = if foldable && folded {
        &steps[steps.len() - 1..]
    } else {
        steps
    };
    let mut block = col()
        .key(format!("steps-{key}"))
        .role("list")
        .label(tr("Steps"))
        .gap(2.0)
        .pl(10.0)
        .border_sides([0.0, 0.0, 0.0, p.hair.max(1.0)], p.k.c.border_weak);
    if foldable {
        let toggle_key = key.to_owned();
        block = block.child(
            row()
                .key("fold")
                .role("button")
                .label(tr_fill("{} steps", &steps.len().to_string()))
                .expanded(!folded)
                .focusable(true)
                .focus_ring(2.0, 0.0, p.k.c.border_focus)
                .gap(4.0)
                .h(22.0)
                .center_y()
                .self_start()
                .pr(6.0)
                .radius(p.k.radius.sm)
                .hover(p.k.c.hover)
                .on_press(move |ui: &mut Ui| {
                    let unfolded = &mut ui.chats.unfolded;
                    match unfolded.iter().position(|open| *open == toggle_key) {
                        Some(index) => {
                            unfolded.remove(index);
                        }
                        None => unfolded.push(toggle_key.clone()),
                    }
                })
                .child(
                    icon(if folded { &FOLDED } else { &UNFOLDED }, 14.0)
                        .key("chevron")
                        .color(p.k.c.icon_faint)
                        .a11y_hidden(),
                )
                .child(
                    text(tr_fill("{} steps", &steps.len().to_string()))
                        .key("label")
                        .font(p.k.ty.small_medium)
                        .color(p.muted())
                        .a11y_hidden(),
                ),
        );
    }
    for (index, step) in shown.iter().enumerate() {
        block = block.child(step_row(p, index, step, p.muted()));
    }
    block
}

fn step_row(p: Paint, index: usize, step: &str, ink: rune::Color) -> El {
    row()
        .key(format!("step{index}"))
        .role("listitem")
        .label(step.to_owned())
        .gap(6.0)
        .items_start()
        .child(
            icon(AGENT_ICON, 12.0)
                .key("icon")
                .color(p.k.c.icon_faint)
                .mt(2.0)
                .a11y_hidden(),
        )
        .child(
            text(step.to_owned())
                .key("words")
                .font(p.k.ty.small)
                .color(ink)
                .wrap()
                .grow(1.0)
                .shrink()
                .a11y_hidden(),
        )
}

/// A request in flight: its steps, its answer so far, and what it's doing
/// with Stop.
fn in_flight(pane: &ChatsPane, p: Paint, id: &str, turn: &Turn) -> El {
    let mut block = col().key("in-flight").gap(10.0);
    if !turn.steps.is_empty() {
        let key = format!("{id}/live");
        let folded = !pane.unfolded.contains(&key);
        let steps: Vec<&str> = turn.steps.iter().map(String::as_str).collect();
        block = block.child(step_block(p, &key, &steps, folded));
    }
    if !turn.answer.is_empty() {
        block = block.child(markdown(p, "streaming", &turn.answer, true));
    }
    let doing = match turn.phase {
        Phase::Sending => tr("Sending…"),
        Phase::Queued => tr("Waiting for a free slot"),
        Phase::Thinking => tr("Thinking…"),
        Phase::Working => tr("Working…"),
        Phase::Answering => tr("Writing…"),
    };
    block.child(
        row()
            .key("status")
            .role("status")
            .label(doing)
            .gap(8.0)
            .h(24.0)
            .center_y()
            .child(Spinner::new().size(14.0).color(p.accent()).build(p.k).a11y_hidden())
            .child(
                text(doing)
                    .key("doing")
                    .font(DESCRIPTION)
                    .color(p.muted())
                    .a11y_hidden(),
            ),
    )
}

fn error_row(p: Paint, key: &str, message: &str) -> El {
    row()
        .key(key.to_owned())
        .role("status")
        .label(message.to_owned())
        .gap(8.0)
        .items_start()
        .px(10.0)
        .py(8.0)
        .radius(p.k.radius.md)
        .fill(p.k.c.danger.bg)
        .border(0.5, p.k.c.danger.border)
        .child(
            icon(ERROR_ICON, 14.0)
                .key("icon")
                .color(p.danger())
                .mt(1.0)
                .a11y_hidden(),
        )
        .child(
            text(message.to_owned())
                .key("words")
                .font(DESCRIPTION)
                .color(p.text())
                .wrap()
                .grow(1.0)
                .shrink()
                .a11y_hidden(),
        )
}

/// A screenshot the conversation saved, as a thumbnail that opens it.
fn screenshot(ui: &Ui, p: Paint, id: &str, index: usize, file: &str) -> El {
    let path = crate::assistant::attachment(id, file);
    let image = ui.chats.images.get(path.as_ref());
    let ready = image
        .as_ref()
        .filter(|image| image.is_ready() && image.error().is_none());
    let height = ready
        .and_then(rune::Image::size)
        .map_or(THUMB_W * 0.625, |[w, h]| {
            (THUMB_W * h as f32 / w.max(1) as f32).clamp(40.0, 320.0)
        });
    let mut thumb = el()
        .key(format!("shot{index}"))
        .role("button")
        .label(tr("Open screenshot"))
        .focusable(true)
        .focus_ring(2.0, 0.0, p.k.c.border_focus)
        .size(THUMB_W, height)
        .radius(p.k.radius.md)
        .fill(p.k.c.layer_01)
        .border(p.hair, p.k.c.border_weak)
        .clip_soft(0.0);
    if let Some(image) = ready {
        thumb = thumb.pixels_fit(image.pixels(), rune::SurfaceFit::Fill);
    }
    if ui.chats.live()
        && let Some(path) = path
    {
        thumb = thumb.on_press(move |_: &mut Ui| {
            if let Err(error) = std::process::Command::new("/usr/bin/open").arg(&path).spawn() {
                tracing::warn!(%error, "could not open a chat's screenshot");
            }
        });
    }
    thumb
}

/// The composer: Return sends, Shift-Return starts a new line. While the
/// conversation runs and nothing is typed, its button stops it.
fn composer(ui: &Ui, p: Paint) -> El {
    let pane = &ui.chats;
    let empty = pane.composer.text.trim().is_empty();
    let placeholder = if pane.selected.is_some() {
        tr("Reply…")
    } else {
        tr("Ask anything…")
    };
    let field = TextArea::new(&pane.composer)
        .placeholder(placeholder)
        .label(tr("Message"))
        .width(None)
        .auto_grow(1, 8)
        .on_edit(|ui: &mut Ui| &mut ui.chats.composer)
        .on_key(|ui: &mut Ui, event: &rune::KeyEvent, tx: &mut Tx| {
            if event.key != rune::Key::Enter || event.mods.shift || event.mods.alt {
                return false;
            }
            if ui.chats.send() {
                tx.instant();
            }
            true
        })
        .build(p.k)
        .key("composer-input")
        .grow(1.0)
        .shrink();
    let running = pane
        .selected
        .as_deref()
        .is_some_and(|id| pane.running(id));
    let action = if running && empty {
        let stop = pane.selected.clone().unwrap_or_default();
        IconButton::new(STOP_ICON)
            .variant(IconButtonVariant::Neutral)
            .control_size(ControlSize::Large)
            .label(tr("Stop"))
            .on_press(move |ui: &mut Ui| ui.chats.stop(&stop))
            .build(p.k)
            .key("composer-stop")
    } else {
        IconButton::new(SEND_ICON)
            .variant(IconButtonVariant::Submit)
            .control_size(ControlSize::Large)
            .label(tr("Send"))
            .disabled(empty)
            .on_press_tx(|ui: &mut Ui, tx: &mut Tx| {
                if ui.chats.send() {
                    tx.instant();
                }
            })
            .build(p.k)
            .key("composer-send")
    };
    row()
        .key("composer")
        .gap(8.0)
        .items_end()
        .pt(10.0)
        .pb(16.0)
        .pr(12.0)
        .child(field)
        .child(action)
}
