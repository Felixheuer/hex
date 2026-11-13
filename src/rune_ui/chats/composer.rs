//! The bottom of the stream: the interrupted banner with Resume, and the
//! composer with its Here / Background toggle. The composer asks the
//! assistant anew, which knows the whole stream; after a click on an entry
//! it replies to that entry, and while that one runs, steers it. Over the
//! stream float "Jump to current" (activity elsewhere
//! while the person stays put) and "↓ New" (more arrived below while they
//! scrolled up).

use rune::{Align, El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};

use super::super::controls::{DESCRIPTION, Paint, PushSize};
use super::super::onboarding_kit::arrives;
use super::list::INTERRUPTED_ICON;
use super::{SendMode, Ui, follow};
use crate::assistant::Outcome;
use crate::desktop::i18n::{tr, tr_fill};

static SEND_ICON: &rune::Icon = &rune_theme_opencode::icons::ARROW_UP;
static STOP_ICON: &rune::Icon = &rune_theme_opencode::icons::STOP;
static DOWN_ICON: &rune::Icon = &rune_theme_opencode::icons::ARROW_DOWN_TO_LINE;
static CLEAR_ICON: &rune::Icon = &rune_theme_opencode::icons::CLOSE_SMALL;

/// Agent-Desk quit or crashed while conversation `id` ran: say so, with Resume.
pub(super) fn interrupted_banner(ui: &Ui, p: Paint, id: &str) -> Option<El> {
    let pane = &ui.chats;
    let interrupted = pane
        .list
        .iter()
        .any(|row| row.id == id && row.outcome == Outcome::Interrupted);
    if !interrupted || pane.running(id) {
        return None;
    }
    let c = &p.k.c;
    let resume = id.to_owned();
    Some(
        row()
            .key("interrupted")
            .role("status")
            .label(tr("This task stopped when Agent-Desk quit"))
            .mr(12.0)
            .mt(8.0)
            .pl(12.0)
            .pr(8.0)
            .py(8.0)
            .gap(10.0)
            .center_y()
            .radius(p.k.radius.md)
            .fill(c.warning.bg)
            .border(0.5, c.warning.border)
            .transition(arrives())
            .child(
                icon(INTERRUPTED_ICON, 15.0)
                    .key("icon")
                    .color(p.warning())
                    .a11y_hidden(),
            )
            .child(
                col()
                    .key("copy")
                    .grow(1.0)
                    .shrink()
                    .gap(2.0)
                    .child(
                        text(tr("This task stopped when Agent-Desk quit"))
                            .key("title")
                            .font(p.k.ty.ui_medium)
                            .color(p.text())
                            .a11y_hidden(),
                    )
                    .child(
                        text(tr(
                            "Resume checks what it already did, then carries on from there.",
                        ))
                        .key("detail")
                        .font(DESCRIPTION)
                        .color(p.muted())
                        .wrap()
                        .a11y_hidden(),
                    ),
            )
            .child(
                Button::new(tr("Resume"))
                    .variant(ButtonVariant::Contrast)
                    .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
                        tx.instant();
                        ui.chats.resume(&resume);
                    })
                    .build_small_push(p.k)
                    .key("resume"),
            ),
    )
}

/// The pills floating over the transcript: "Jump to current" at its top,
/// "↓ New" at its bottom.
pub(super) fn pills(ui: &Ui, p: Paint) -> Vec<El> {
    let pane = &ui.chats;
    let mut pills = Vec::new();
    if let Some(elsewhere) = &pane.follow.elsewhere {
        let title = pane
            .task(elsewhere)
            .map(|task| task.title.clone())
            .or_else(|| pane.turns.get(elsewhere).map(|turn| turn.question.clone()))
            .unwrap_or_default();
        pills.push(
            row()
                .key("jump-slot")
                .pin(Some(0.0), Some(4.0), Some(0.0), None)
                .justify_center()
                .child(
                    pill(p, "jump-to-current", tr("Jump to current"), &title, None).on_press_tx(
                        |ui: &mut Ui, tx: &mut Tx| {
                            tx.instant();
                            ui.chats.jump_to_current();
                            if let Some(id) = &ui.chats.selected {
                                tx.scroll_into_view(super::stream::entry_key(id));
                            }
                        },
                    ),
                ),
        );
    }
    if pane.follow.unseen {
        pills.push(
            row()
                .key("new-slot")
                .pin(Some(0.0), None, Some(0.0), Some(8.0))
                .justify_center()
                .child(
                    pill(p, "new-below", tr("New"), "", Some(DOWN_ICON)).on_press_tx(
                        |ui: &mut Ui, tx: &mut Tx| {
                            ui.chats.follow.unseen = false;
                            tx.scroll_to_bottom(follow::SCROLL);
                        },
                    ),
                ),
        );
    }
    pills
}

/// A small raised pill: an icon, a label, and a quiet detail.
fn pill(p: Paint, key: &str, label: &str, detail: &str, glyph: Option<&'static rune::Icon>) -> El {
    let k = p.k;
    let mut pill = row()
        .key(key.to_owned())
        .role("button")
        .label(label.to_owned())
        .focusable(true)
        .focus_ring(2.0, 0.0, k.c.border_focus)
        .h(28.0)
        .px(12.0)
        .gap(6.0)
        .center_y()
        .max_w(360.0)
        .radius(14.0)
        .fill(k.c.surface_raised)
        .border(p.hair, k.c.border_weak)
        .hover(k.c.hover)
        .transition(arrives());
    for shadow in &k.elevation.raised {
        pill = pill.box_shadow(*shadow);
    }
    pill.children(glyph.map(|glyph| icon(glyph, 13.0).key("icon").color(p.muted()).a11y_hidden()))
        .child(
            text(label.to_owned())
                .key("label")
                .font(k.ty.small_medium)
                .color(p.text())
                .a11y_hidden(),
        )
        .children((!detail.is_empty()).then(|| {
            text(detail.to_owned())
                .key("detail")
                .font(k.ty.small)
                .color(p.muted())
                .shrink()
                .truncate()
                .a11y_hidden()
        }))
}

/// Where a new message goes: Auto (the assistant decides), with Here and
/// Background as overrides in a small menu.
fn send_mode(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    const MODES: [SendMode; 3] = [SendMode::Auto, SendMode::Here, SendMode::Background];
    let labels = vec![
        tr("Auto").to_owned(),
        tr("Here: the window in front").to_owned(),
        tr("Background job").to_owned(),
    ];
    let selected = MODES
        .iter()
        .position(|mode| *mode == ui.chats.lane)
        .unwrap_or(0);
    super::super::general::quiet_select(
        ui,
        p,
        super::super::Menu::Pick("composer-send-mode"),
        "composer-send-mode-control".to_owned(),
        tr("Send as"),
        labels,
        selected,
        |ui, chosen| ui.chats.lane = MODES[chosen.min(MODES.len() - 1)],
        layer,
    )
}

/// The composer: Return sends, Shift-Return starts a new line. While the
/// shown conversation runs, an empty composer's button stops it and a
/// message steers it (or answers what it asked).
pub(super) fn composer(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let pane = &ui.chats;
    let empty = pane.composer.text.trim().is_empty();
    let running = pane.target.as_deref().is_some_and(|id| pane.running(id));
    let placeholder = match (running, pane.target.is_some(), pane.lane) {
        (true, _, _) => tr("Tell it something while it works…"),
        (false, true, _) => tr("Reply…"),
        (false, false, SendMode::Here) => tr("Ask about the window in front…"),
        (false, false, SendMode::Auto | SendMode::Background) => {
            tr("Ask anything, or give it a job…")
        }
    };
    // The bare editor (what TextArea draws inside its frame): the rounded
    // box below is the frame.
    let ink = p.k.c.text_base;
    let style = rune::TextStyle {
        line_height: (p.k.ty.body.size * 1.45 * 2.0).round() / 2.0,
        ..p.k.ty.body
    };
    let input = rune::text_input(&pane.composer, style)
        .key("input")
        .w_full()
        .color(ink)
        .placeholder(placeholder, p.k.c.text_faint)
        .caret(ink, p.k.selection())
        .label(tr("Message"))
        .on_edit(|ui: &mut Ui| &mut ui.chats.composer)
        .on_input_key(|ui: &mut Ui, event: &rune::KeyEvent, tx: &mut Tx| {
            if event.key != rune::Key::Enter || event.mods.shift || event.mods.alt {
                return false;
            }
            if ui.chats.send() {
                tx.instant();
            }
            true
        });
    let field = col()
        .key("composer-input")
        .role("textbox")
        .w_full()
        .px(8.0)
        .pt(8.0)
        .pb(4.0)
        .child(rune::auto_grow(input, 1, 8));
    let action = if running && empty {
        let stop = pane.target.clone().unwrap_or_default();
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
            .label(if running {
                tr("Send to this task")
            } else {
                tr("Send")
            })
            .disabled(empty)
            .on_press_tx(|ui: &mut Ui, tx: &mut Tx| {
                if ui.chats.send() {
                    tx.instant();
                }
            })
            .build(p.k)
            .key("composer-send")
    };
    // Beside the button: where a new message goes, or, while it runs, that
    // a message reaches the task as it works.
    let lead = if running {
        text(tr("It hears you at its next step"))
            .key("steer-hint")
            .font(p.k.ty.small)
            .color(p.faint())
            .align(Align::Start)
            .shrink()
            .truncate()
    } else {
        send_mode(ui, p, layer)
    };
    let controls = row()
        .key("composer-controls")
        .gap(8.0)
        .center_y()
        .child(lead)
        .child(el().key("spacer").grow(1.0))
        .child(action);
    // One rounded box, as in a chat app: the field without its own frame,
    // the controls along its bottom edge; its border lights while typing.
    let k = p.k;
    let focused = pane.composer.focused;
    let shell = col()
        .key("composer-box")
        .w_full()
        .max_w(super::transcript::READING_W)
        .gap(2.0)
        .pl(6.0)
        .pr(8.0)
        .pt(6.0)
        .pb(8.0)
        .radius(k.radius.lg + 10.0)
        .fill(k.c.layer_01)
        .border(
            p.hair.max(1.0),
            if focused {
                k.c.border_strong
            } else {
                k.c.border_weak
            },
        )
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 4.0,
            blur: 18.0,
            spread: 0.0,
            color: rune::Color::BLACK.alpha(if k.dark { 0.28 } else { 0.06 }),
            inset: false,
        })
        .children(target_chip(ui, p))
        .child(field)
        .child(controls.pl(6.0));
    col()
        .key("composer")
        .w_full()
        .items_center()
        .pt(10.0)
        .pb(16.0)
        .pr(12.0)
        .child(shell)
}

/// The entry the composer writes to, after a click on it, with a way back
/// to writing to the assistant.
fn target_chip(ui: &Ui, p: Paint) -> Option<El> {
    let pane = &ui.chats;
    let id = pane.target.as_deref()?;
    let title = super::stream::title(pane, id);
    Some(
        row()
            .key("composer-target")
            .gap(4.0)
            .pl(8.0)
            .center_y()
            .child(
                text(tr_fill("To: {}", &title))
                    .key("to")
                    .font(p.k.ty.small)
                    .color(p.accent())
                    .shrink()
                    .truncate(),
            )
            .child(
                IconButton::new(CLEAR_ICON)
                    .variant(IconButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .label(tr("Write to the assistant instead"))
                    .on_press_tx(|ui: &mut Ui, tx: &mut Tx| {
                        ui.chats.clear_target();
                        tx.focus("composer-input", false);
                    })
                    .build(p.k)
                    .key("clear-target"),
            ),
    )
}
