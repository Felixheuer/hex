//! One of a task's browsers, live and large, over Chats: opened from its
//! preview in the orb's list. Below it, Take over hands the task to the
//! person (its browser and desktop actions wait) and lets them click,
//! scroll, and type in the page; Hand back gives it to the agent again,
//! which looks afresh before it acts.

use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};

use super::super::assistant_overlay::live::{self, Preview};
use super::super::controls::{Paint, PushSize};
use super::{Hold, Ui};
use crate::assistant::browser::stream::{Button as Mouse, Input};
use crate::desktop::i18n::tr;

/// The picture's largest size, in points.
const MAX_W: f32 = 880.0;
const MAX_H: f32 = 560.0;
/// The page's size when no frame has said it yet.
const DEFAULT_VIEWPORT: (f32, f32) = (1280.0, 800.0);

/// Opens `session` of `task` large.
pub(super) fn open(ui: &mut Ui, task: String, session: String) {
    live::view(Some((&task, &session)));
    ui.chats.live = Some((task, session));
}

/// Closes the large view; the stream goes back to small, or stops.
pub(super) fn close(ui: &mut Ui) -> bool {
    live::view(None);
    ui.chats.live.take().is_some()
}

/// The picture's size for a page of `viewport`, filling the room it has
/// and keeping the page's proportions.
pub(super) fn fitted(viewport: (f32, f32)) -> (f32, f32) {
    let (vw, vh) = if viewport.0 > 0.0 && viewport.1 > 0.0 {
        viewport
    } else {
        DEFAULT_VIEWPORT
    };
    let scale = (MAX_W / vw).min(MAX_H / vh);
    (vw * scale, vh * scale)
}

/// The sheet, while a browser shows large.
pub(super) fn sheet(ui: &Ui, p: Paint) -> Option<El> {
    let (task, session) = ui.chats.live.clone()?;
    let k = p.k;
    let preview = live::preview(&task, &session);
    let page = preview
        .as_ref()
        .map(|preview| preview.page())
        .unwrap_or_default();
    let status = ui.chats.task(&task);
    let held = status.is_some_and(|status| matches!(ui.chats.hold(status), Some(Hold::Paused)));
    let viewport = if page.viewport.0 > 0.0 {
        page.viewport
    } else {
        DEFAULT_VIEWPORT
    };
    let (w, h) = fitted(viewport);
    let title = if !page.title.is_empty() {
        page.title.clone()
    } else {
        preview
            .as_ref()
            .filter(|preview| !preview.name.is_empty())
            .map_or_else(
                || tr("Main browser").to_owned(),
                |preview| preview.name.clone(),
            )
    };
    let header = row()
        .key("live-header")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            col()
                .key("live-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(title)
                        .key("live-title")
                        .role("heading")
                        .truncate()
                        .font(k.ty.ui_medium)
                        .color(p.text()),
                )
                .children((!page.url.is_empty()).then(|| {
                    text(page.url.clone())
                        .key("live-url")
                        .truncate()
                        .font(k.ty.small)
                        .color(p.faint())
                })),
        )
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .label(tr("Close"))
                .on_press(|ui: &mut Ui| {
                    close(ui);
                })
                .build(k)
                .key("live-close"),
        );
    let picture = picture(
        p,
        preview.as_deref(),
        &task,
        &session,
        (w, h),
        viewport,
        held,
    );
    let (words, action) = if status.is_none() {
        (tr("This task has finished."), None)
    } else if held {
        let id = task.clone();
        (
            tr("You're driving: click, scroll, and type in the page. The task waits."),
            Some(
                Button::new(tr("Hand back"))
                    .variant(ButtonVariant::Contrast)
                    .on_press(move |ui: &mut Ui| ui.chats.hand_back(&id))
                    .build_small_push(k)
                    .key("live-hand-back"),
            ),
        )
    } else {
        let id = task.clone();
        (
            tr("The assistant is working here. Take over to drive it yourself."),
            Some(
                Button::new(tr("Take over"))
                    .variant(ButtonVariant::Outline)
                    .on_press(move |ui: &mut Ui| ui.chats.take_over(&id))
                    .build_small_push(k)
                    .key("live-take-over"),
            ),
        )
    };
    let bar = row()
        .key("live-bar")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            text(words)
                .key("live-status")
                .role("status")
                .font(k.ty.small)
                .color(p.muted())
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
        .children(action);
    let card = col()
        .key("live-card")
        .role("dialog")
        .label(tr("Live browser"))
        .w(w + 32.0)
        .padding(16.0)
        .gap(12.0)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .border(p.hair, k.c.border_weak)
        .on_press(|_: &mut Ui| {})
        .child(header)
        .child(picture)
        .child(bar);
    Some(
        el().key("live-sheet")
            // A scrim, not a control: Close and Escape close it too.
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(k.c.bg_deep.alpha(0.6))
            .on_press(|ui: &mut Ui| {
                close(ui);
            })
            .child(card)
            .modal(true, |ui: &mut Ui| {
                close(ui);
            }),
    )
}

/// The live picture at `size`; while the person drives (`held`), it takes
/// their pointer, wheel, and keys to the page.
fn picture(
    p: Paint,
    preview: Option<&Preview>,
    task: &str,
    session: &str,
    size: (f32, f32),
    viewport: (f32, f32),
    held: bool,
) -> El {
    let k = p.k;
    let base = el()
        .key("live-picture")
        .size(size.0, size.1)
        .radius(8.0)
        .fill(k.c.layer_02)
        .border(p.hair, k.alpha(k.c.text_base, 0.14));
    let base = match preview.filter(|preview| preview.has_frame()) {
        Some(preview) => base.pixels_fit(&preview.pixels, rune::SurfaceFit::Fit),
        None => base.center().child(
            text(match preview.map(Preview::page) {
                Some(page) if page.failed.is_some() => tr("No live picture of this browser"),
                _ => tr("Loading…"),
            })
            .key("live-waiting")
            .font(k.ty.small)
            .color(p.faint()),
        ),
    };
    if !held {
        return base;
    }
    let (drag_task, drag_session) = (task.to_owned(), session.to_owned());
    let (move_task, move_session) = (task.to_owned(), session.to_owned());
    let (wheel_task, wheel_session) = (task.to_owned(), session.to_owned());
    let (key_task, key_session) = (task.to_owned(), session.to_owned());
    base.role("application")
        .label(tr("Live browser you're driving"))
        .focusable(true)
        .hit()
        .on_drag(move |_: &mut Ui, drag: &mut rune::Drag, _| {
            let (x, y) = live::to_page((drag.x, drag.y), (drag.w, drag.h), viewport);
            let clicks = drag.clicks.max(1);
            let input = match drag.phase {
                rune::Phase::Down => Input::Press {
                    x,
                    y,
                    button: Mouse::Left,
                    clicks,
                },
                rune::Phase::Move => Input::Move { x, y },
                rune::Phase::Up => Input::Release {
                    x,
                    y,
                    button: Mouse::Left,
                    clicks,
                },
            };
            live::send(&drag_task, &drag_session, &input);
        })
        .on_pointer_move_local(move |_: &mut Ui, pointer: rune::PointerEvent, tx| {
            let (x, y) = live::to_page((pointer.x, pointer.y), size, viewport);
            live::send(&move_task, &move_session, &Input::Move { x, y });
            tx.live_only();
        })
        .on_wheel(move |_: &mut Ui, wheel: rune::WheelEvent, _| {
            let (x, y) = live::to_page((wheel.x, wheel.y), size, viewport);
            // The wheel moves content with the fingers; a page scrolls the
            // other way.
            live::send(
                &wheel_task,
                &wheel_session,
                &Input::Wheel {
                    x,
                    y,
                    dx: -wheel.dx,
                    dy: -wheel.dy,
                },
            );
            true
        })
        .on_key(move |_: &mut Ui, key: &rune::KeyEvent, _| {
            match live::key_input(&key.key, key.mods) {
                Some(input) => {
                    live::send(&key_task, &key_session, &input);
                    true
                }
                None => false,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_keeps_the_pages_proportions() {
        assert_eq!(fitted((1280.0, 720.0)), (880.0, 495.0));
        assert_eq!(fitted((800.0, 1200.0)), (373.33334, 560.0));
        assert_eq!(fitted((0.0, 0.0)), (880.0, 550.0));
    }
}
