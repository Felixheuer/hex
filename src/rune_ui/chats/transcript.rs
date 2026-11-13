//! How the stream draws what was said: questions on the right, answers as
//! Markdown, what the assistant did as compact muted steps, screenshots,
//! errors, the request in flight with its answer streaming and what was
//! sent while it worked; and the welcome and finish-setup cards.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use rune::{El, Waker, col, el, icon, row, text};
use rune_kit::{Markdown, Spinner};

use super::super::controls::{DESCRIPTION, Paint};
use super::super::list_detail::{FOLDED, UNFOLDED};
use super::list::AGENT_ICON;
use super::{Hold, Phase, Turn, Ui};
use crate::assistant::{Conversation, Message, Part};
use crate::desktop::i18n::{tr, tr_fill};

static ERROR_ICON: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_X;

/// The widest a question's bubble grows.
const BUBBLE_MAX: f32 = 520.0;
/// The reading column: a transcript, its header, and the composer keep to
/// this width, centred, so lines stay comfortable on a wide window.
pub(super) const READING_W: f32 = 760.0;
/// A screenshot thumbnail's width.
const THUMB_W: f32 = 320.0;
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

/// Setup isn't finished: a calm card that picks it up where it was left.
pub(super) fn finish_setup(ui: &Ui, p: Paint) -> El {
    use super::super::onboarding_kit::{LEAD, title_font};
    let saved = super::setup::resume::saved_page(ui);
    let detail = match saved {
        Some(page) if page != super::setup::Page::Welcome => {
            tr("It picks up where you left off, and checks that page again.")
        }
        _ => tr(
            "Hex checks that the agent, its permissions, and the shortcuts work, then you try it once.",
        ),
    };
    col()
        .key("finish-setup")
        .grow(1.0)
        .shrink()
        .center()
        .gap(10.0)
        .pb(40.0)
        .child(
            text(tr("Finish setting up the assistant"))
                .key("title")
                .role("heading")
                .level(2)
                .font(title_font(p))
                .color(p.text()),
        )
        .child(
            text(detail)
                .key("detail")
                .font(LEAD)
                .color(p.muted())
                .align(rune::Align::Center)
                .wrap()
                .max_w(420.0),
        )
        .child(
            rune_kit::Button::new(tr("Continue setup"))
                .variant(rune_kit::ButtonVariant::Contrast)
                .on_press(super::open_setup)
                .build(p.k)
                .key("continue-setup")
                .mt(8.0),
        )
        .transition(super::super::onboarding_kit::arrives())
}

/// Nothing selected: what the assistant is for, over the composer.
pub(super) fn new_chat(ui: &Ui, p: Paint) -> El {
    use super::super::onboarding_kit::{LEAD, keys, title_font};
    let hint = match ui.settings.assistant.action_hotkey.as_ref() {
        Some(binding) => {
            let caps = super::super::settings::shortcut_caps(binding, false);
            row()
                .key("hint")
                .gap(8.0)
                .center_y()
                .child(
                    text(tr("Type below, or hold"))
                        .key("type")
                        .font(LEAD)
                        .color(p.muted()),
                )
                .child(keys(p, "keys", &caps, false))
                .child(
                    text(tr("anywhere"))
                        .key("anywhere")
                        .font(LEAD)
                        .color(p.muted()),
                )
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
        .child(suggestions(p))
        .transition(super::super::onboarding_kit::arrives())
}

/// What to ask first: each fills the composer, ready to send or change.
const SUGGESTIONS: [&str; 4] = [
    "Summarize the page in my browser",
    "Research the best options and write me a short report",
    "Sort my Downloads folder by type",
    "Every weekday at 8, summarize my inbox",
];

fn suggestions(p: Paint) -> El {
    let k = p.k;
    let chip = |index: usize, words: &'static str| {
        rune_kit::Button::new(tr(words))
            .variant(rune_kit::ButtonVariant::Outline)
            .on_press_tx(move |ui: &mut Ui, tx: &mut rune::Tx| {
                ui.chats.composer = rune::TextEdit::new(tr(words));
                tx.focus("composer-input", false);
            })
            .build(k)
            .key(format!("suggestion{index}"))
            .radius(k.radius.lg + 8.0)
    };
    row()
        .key("suggestions")
        .role("list")
        .label(tr("Suggestions"))
        .mt(18.0)
        .gap(8.0)
        .flex_wrap()
        .justify_center()
        .max_w(READING_W)
        .children(
            SUGGESTIONS
                .iter()
                .enumerate()
                .map(|(index, words)| chip(index, words)),
        )
}

/// `child` centred in the full width, at most [`READING_W`] wide.
pub(super) fn centred(key: &str, child: El) -> El {
    col()
        .key(key.to_owned())
        .w_full()
        .items_center()
        .child(child)
}

pub(super) fn ends_in_error(conversation: &Conversation) -> bool {
    conversation
        .messages
        .last()
        .and_then(|message| message.parts.last())
        .is_some_and(|part| matches!(part, Part::Error { .. }))
}

/// A question: its words in a bubble on the right, screenshots under it.
pub(super) fn question(ui: &Ui, p: Paint, id: &str, index: usize, message: &Message) -> El {
    let words = message.text();
    let mut block = col().key(format!("message{index}")).gap(8.0).items_end();
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
pub(super) fn bubble(p: Paint, key: &str, words: &str) -> El {
    row().key(key.to_owned()).justify_end().child(
        text(words.to_owned())
            .key("words")
            .font(p.k.ty.body)
            .color(p.text())
            .selectable(true)
            .wrap()
            .max_w(BUBBLE_MAX)
            .px(16.0)
            .py(10.0)
            .radius(p.k.radius.lg + 8.0)
            .fill(p.k.c.layer_02),
    )
}

/// An answer: its parts in order, a run of actions as one step block.
pub(super) fn answer(ui: &Ui, p: Paint, id: &str, index: usize, message: &Message) -> El {
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
    let k = p.k;
    Markdown::new(source.to_owned())
        .streaming(streaming)
        .selectable(true)
        .scroll_wide(true)
        .code_frame(move |card, code| code_card(k, card, code.index, code.source, code.open))
        .build(p.k)
        .key(key.to_owned())
}

/// A code block with Copy in its corner (once it has fully arrived).
fn code_card(k: &'static rune_kit::Kit, card: El, index: usize, source: &str, open: bool) -> El {
    let copy = (!open).then(|| {
        let source = source.to_owned();
        rune_kit::IconButton::new(super::super::list_detail::COPY_ICON)
            .variant(rune_kit::IconButtonVariant::GhostMuted)
            .control_size(rune_kit::ControlSize::Small)
            .label(tr("Copy code"))
            .on_press_tx(move |_: &mut Ui, tx: &mut rune::Tx| tx.copy(source.clone()))
            .build(k)
            .key("copy")
    });
    el().key(format!("code-{index}"))
        .w_full()
        .child(card)
        .children(copy.map(|copy| {
            row()
                .key("corner")
                .absolute(0.0, 0.0)
                .w_full()
                .justify_end()
                .padding(6.0)
                .child(copy)
        }))
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
        .gap(3.0)
        .w_full()
        .px(10.0)
        .py(6.0)
        .radius(p.k.radius.md + 2.0)
        .fill(p.k.alpha(p.k.c.text_base, if p.k.dark { 0.035 } else { 0.03 }));
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
pub(super) fn in_flight(ui: &Ui, p: Paint, id: &str, turn: &Turn) -> El {
    let pane = &ui.chats;
    let mut block = col().key("in-flight").gap(10.0);
    if !turn.steps.is_empty() {
        let key = format!("{id}/live");
        let folded = !pane.unfolded.contains(&key);
        let steps: Vec<&str> = turn.steps.iter().map(String::as_str).collect();
        block = block.child(step_block(p, &key, &steps, folded));
    }
    for (index, (question, answer)) in turn.answers.iter().enumerate() {
        block = block.child(super::work::answered(p, index, question, answer));
    }
    for (index, words) in turn.steered.iter().enumerate() {
        block = block.child(
            col()
                .key(format!("steered{index}"))
                .gap(4.0)
                .items_end()
                .child(bubble(p, "bubble", words))
                .child(
                    text(tr("Sent while working"))
                        .key("caption")
                        .font(p.k.ty.small)
                        .color(p.faint()),
                ),
        );
    }
    if !turn.answer.is_empty() {
        block = block.child(markdown(p, "streaming", &turn.answer, true));
    }
    if let Some(question) = super::work::question_block(ui, p, id) {
        return block.child(question);
    }
    let doing = match pane.task(id).and_then(|task| pane.hold(task)) {
        Some(Hold::Asking(_)) => tr("Waiting for your answer").to_owned(),
        Some(Hold::Paused) => tr("Paused while you have it. Hand it back to go on").to_owned(),
        Some(Hold::Waiting { until_ms, .. }) => tr_fill(
            "Hit a usage limit. Retries in {}",
            &super::strip::countdown(until_ms, crate::history::now_ms()),
        ),
        None => match turn.phase {
            Phase::Sending => tr("Sending…"),
            Phase::Queued => tr("Waiting for a free slot"),
            Phase::Thinking => tr("Thinking…"),
            Phase::Working => tr("Working…"),
            Phase::Answering => tr("Writing…"),
        }
        .to_owned(),
    };
    let doing = doing.as_str();
    block.child(
        row()
            .key("status")
            .role("status")
            .label(doing)
            .gap(8.0)
            .h(24.0)
            .center_y()
            .child(
                Spinner::new()
                    .size(14.0)
                    .color(p.accent())
                    .build(p.k)
                    .a11y_hidden(),
            )
            .child(
                text(doing)
                    .key("doing")
                    .font(DESCRIPTION)
                    .color(p.muted())
                    .a11y_hidden(),
            ),
    )
}

pub(super) fn error_row(p: Paint, key: &str, message: &str) -> El {
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
            if let Err(error) = std::process::Command::new("/usr/bin/open")
                .arg(&path)
                .spawn()
            {
                tracing::warn!(%error, "could not open a chat's screenshot");
            }
        });
    }
    thumb
}
