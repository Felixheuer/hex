//! The bottom of the detail card: the interrupted banner with Resume, and
//! the composer with its Here / Background toggle. The composer sends into
//! the conversation that shows; while that one runs, what it sends steers
//! it. Over the transcript float "Jump to current" (activity elsewhere
//! while the person stays put) and "↓ New" (more arrived below while they
//! scrolled up).

use rune::{Align, El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, TextArea};

use super::super::controls::{DESCRIPTION, Paint, PushSize};
use super::super::icons::{MIC, SPARKLES};
use super::super::onboarding_kit::arrives;
use super::list::INTERRUPTED_ICON;
use super::{Ui, follow};
use crate::assistant::{Lane, Outcome};
use crate::desktop::i18n::tr;

static SEND_ICON: &rune::Icon = &rune_theme_opencode::icons::ARROW_UP;
static STOP_ICON: &rune::Icon = &rune_theme_opencode::icons::STOP;
static DOWN_ICON: &rune::Icon = &rune_theme_opencode::icons::ARROW_DOWN_TO_LINE;

/// Hex quit or crashed while conversation `id` ran: say so, with Resume.
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
            .label(tr("This task stopped when Hex quit"))
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
                        text(tr("This task stopped when Hex quit"))
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
                    pill(p, "jump-to-current", tr("Jump to current"), &title, None)
                        .on_press_tx(|ui: &mut Ui, tx: &mut Tx| {
                            tx.instant();
                            ui.chats.jump_to_current();
                        }),
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
fn pill(
    p: Paint,
    key: &str,
    label: &str,
    detail: &str,
    glyph: Option<&'static rune::Icon>,
) -> El {
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
    pill.children(glyph.map(|glyph| {
        icon(glyph, 13.0)
            .key("icon")
            .color(p.muted())
            .a11y_hidden()
    }))
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

/// Here / Background: where a new message goes. Hidden while the shown
/// conversation runs (a message then steers it).
fn lane_toggle(ui: &Ui, p: Paint) -> El {
    let lane = ui.chats.lane;
    let k = p.k;
    let option = |key: &str, glyph: &'static rune::Icon, label: &str, this: Lane, help: &str| {
        let on = lane == this;
        let option = row()
            .key(key.to_owned())
            .role("radio")
            .label(label.to_owned())
            .checked(on)
            .focusable(true)
            .focus_ring(2.0, 0.0, k.c.border_focus)
            .help(help.to_owned())
            .h(24.0)
            .px(8.0)
            .gap(5.0)
            .center_y()
            .radius(k.radius.sm);
        let option = if on {
            option.fill(k.c.bg_base).border(p.hair, k.c.border_weak)
        } else {
            option.hover(k.c.hover)
        };
        option
            .on_press(move |ui: &mut Ui| ui.chats.lane = this)
            .child(
                icon(glyph, 12.0)
                    .key("icon")
                    .color(if on { p.text() } else { p.faint() })
                    .a11y_hidden(),
            )
            .child(
                text(label.to_owned())
                    .key("label")
                    .font(k.ty.small_medium)
                    .color(if on { p.text() } else { p.muted() })
                    .a11y_hidden(),
            )
    };
    row()
        .key("lane")
        .role("radiogroup")
        .label(tr("Send as"))
        .px(2.0)
        .py(2.0)
        .gap(2.0)
        .center_y()
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .child(option(
            "lane-here",
            &MIC,
            tr("Here"),
            Lane::Here,
            tr("About the window in front of Hex; one at a time, answered aloud"),
        ))
        .child(option(
            "lane-background",
            &SPARKLES,
            tr("Background"),
            Lane::Background,
            tr("A task that works beside the others, in windows of its own"),
        ))
}

/// The composer: Return sends, Shift-Return starts a new line. While the
/// shown conversation runs, an empty composer's button stops it and a
/// message steers it.
pub(super) fn composer(ui: &Ui, p: Paint) -> El {
    let pane = &ui.chats;
    let empty = pane.composer.text.trim().is_empty();
    let running = pane.selected.as_deref().is_some_and(|id| pane.running(id));
    let placeholder = match (running, pane.selected.is_some(), pane.lane) {
        (true, _, _) => tr("Tell it something while it works…"),
        (false, true, _) => tr("Reply…"),
        (false, false, Lane::Here) => tr("Ask about the window in front…"),
        (false, false, Lane::Background) => tr("Ask anything…"),
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
        lane_toggle(ui, p)
    };
    let controls = row()
        .key("composer-controls")
        .gap(8.0)
        .center_y()
        .child(lead)
        .child(el().key("spacer").grow(1.0))
        .child(action);
    col()
        .key("composer")
        .gap(8.0)
        .pt(10.0)
        .pb(14.0)
        .pr(12.0)
        .child(field)
        .child(controls)
}
