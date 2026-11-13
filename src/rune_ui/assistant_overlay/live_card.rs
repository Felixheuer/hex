//! One of a task's browsers, live and large: the card both the overlay
//! (beside the orb) and Chats show. Above the picture, the page's title and
//! address with What it did (in the overlay, on the stage) and ×; below it, what the
//! task is doing and Take over / Hand back. While the person drives, the
//! picture takes their pointer, wheel, and keys to the page.

use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};

use super::super::controls::{Paint, PushSize};
use super::live::{self, Preview};
use crate::assistant::browser::stream::{Button as Mouse, Input};
use crate::desktop::i18n::tr;

/// The page's size when no frame has said it yet.
pub(crate) const DEFAULT_VIEWPORT: (f32, f32) = (1280.0, 800.0);
/// The card's padding around the picture.
pub(crate) const PAD: f32 = 16.0;

/// Who has the task the card shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Driver {
    /// The agent works; Take over hands it to the person.
    Agent,
    /// The person took it over: the picture takes their input.
    Person,
    /// The task ended; the picture stays until the card closes.
    Finished,
}

/// What the card's buttons ask of the host.
pub(crate) struct Actions<S> {
    pub(crate) close: fn(&mut S),
    pub(crate) take_over: fn(&mut S, &str),
    pub(crate) hand_back: fn(&mut S, &str),
    /// "What it did" (task, session): its steps on the stage; `None` hides
    /// it (in the Assistant pane itself).
    pub(crate) what_it_did: Option<fn(&mut S, &str, &str)>,
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

/// The picture's size for a page of `viewport` inside `room`, keeping the
/// page's proportions.
pub(crate) fn fitted(viewport: (f32, f32), room: (f32, f32)) -> (f32, f32) {
    let (vw, vh) = if viewport.0 > 0.0 && viewport.1 > 0.0 {
        viewport
    } else {
        DEFAULT_VIEWPORT
    };
    let scale = (room.0 / vw).min(room.1 / vh);
    (vw * scale, vh * scale)
}

/// The card for `session` of `task`, its picture at most `room` big.
pub(in crate::rune_ui) fn card<S: 'static>(
    p: Paint,
    task: &str,
    session: &str,
    driver: Driver,
    room: (f32, f32),
    actions: Actions<S>,
) -> El {
    if let Some(number) = super::app_live::number_of(session) {
        return app_card(p, task, number, driver, room, actions);
    }
    let k = p.k;
    let preview = live::preview(task, session);
    let page = preview
        .as_ref()
        .map(|preview| preview.page())
        .unwrap_or_default();
    let viewport = if page.viewport.0 > 0.0 {
        page.viewport
    } else {
        DEFAULT_VIEWPORT
    };
    let (w, h) = fitted(viewport, room);
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
    let close = actions.close;
    let what_it_did = actions.what_it_did.map(|open| {
        let (task, session) = (task.to_owned(), session.to_owned());
        Button::new(tr("What it did"))
            .variant(ButtonVariant::Ghost)
            .on_press(move |s: &mut S| open(s, &task, &session))
            .build_small_push(k)
            .key("live-what-it-did")
    });
    let header = row()
        .key("live-header")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(live_dot(p, driver))
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
        .children(what_it_did)
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .label(tr("Close"))
                .on_press(move |s: &mut S| close(s))
                .build(k)
                .key("live-close"),
        );
    let picture = picture::<S>(
        p,
        preview.as_deref(),
        task,
        session,
        (w, h),
        viewport,
        driver == Driver::Person,
    );
    let (words, action) = match driver {
        Driver::Finished => (tr("This task has finished."), None),
        Driver::Person => {
            let (id, hand_back) = (task.to_owned(), actions.hand_back);
            (
                tr("You're driving: click, scroll, and type in the page. The task waits."),
                Some(
                    Button::new(tr("Hand back"))
                        .variant(ButtonVariant::Contrast)
                        .on_press(move |s: &mut S| hand_back(s, &id))
                        .build_small_push(k)
                        .key("live-hand-back"),
                ),
            )
        }
        Driver::Agent => {
            let (id, take_over) = (task.to_owned(), actions.take_over);
            (
                tr("The assistant is working here. Take over to drive it yourself."),
                Some(
                    Button::new(tr("Take over"))
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut S| take_over(s, &id))
                        .build_small_push(k)
                        .key("live-take-over"),
                ),
            )
        }
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
    col()
        .key("live-card")
        .role("dialog")
        .label(tr("Live browser"))
        .w(w + PAD * 2.0)
        .padding(PAD)
        .gap(12.0)
        .radius(k.radius.lg + 4.0)
        .fill(k.c.bg_base)
        .border(p.hair, k.c.border_weak)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 12.0,
            blur: 36.0,
            spread: 0.0,
            color: rune::Color::BLACK.alpha(if k.dark { 0.5 } else { 0.18 }),
            inset: false,
        })
        .hit()
        .on_press(|_: &mut S| {})
        .child(header)
        .child(picture)
        .child(bar)
}

/// One of a job's app windows, live and large: its app above, the window
/// (captured even while parked off the screen) in the middle, and below,
/// Take over (the window comes to the person, in front) or Hand back (it
/// goes back off the screen and the job goes on).
fn app_card<S: 'static>(
    p: Paint,
    task: &str,
    number: u32,
    driver: Driver,
    room: (f32, f32),
    actions: Actions<S>,
) -> El {
    let k = p.k;
    let preview = super::app_live::preview(task, number);
    let pixels = preview
        .as_ref()
        .filter(|preview| preview.has_frame())
        .map(|preview| preview.large_pixels().size());
    let shape = pixels
        .map(|(w, h)| (w as f32, h as f32))
        .unwrap_or(DEFAULT_VIEWPORT);
    let (w, h) = fitted(shape, room);
    let app = preview
        .as_ref()
        .map_or_else(|| tr("App window").to_owned(), |p| p.window.app.clone());
    let close = actions.close;
    let header = row()
        .key("live-header")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(live_dot(p, driver))
        .child(
            col()
                .key("live-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(app)
                        .key("live-title")
                        .role("heading")
                        .truncate()
                        .font(k.ty.ui_medium)
                        .color(p.text()),
                )
                .child(
                    text(match driver {
                        Driver::Person => tr("On your screen"),
                        _ => tr("In the background, off your screen"),
                    })
                    .key("live-url")
                    .truncate()
                    .font(k.ty.small)
                    .color(p.faint()),
                ),
        )
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .label(tr("Close"))
                .on_press(move |s: &mut S| close(s))
                .build(k)
                .key("live-close"),
        );
    let base = el()
        .key("live-picture")
        .size(w, h)
        .radius(10.0)
        .fill(k.c.layer_02)
        .border(p.hair, k.alpha(k.c.text_base, 0.14));
    let picture = match preview.as_ref().filter(|preview| preview.has_frame()) {
        Some(preview) => base.pixels_fit(preview.large_pixels(), rune::SurfaceFit::Fit),
        None => base.center().child(
            text(if preview.as_ref().is_some_and(|p| p.failed()) {
                tr("No live picture of this window")
            } else {
                tr("Loading…")
            })
            .key("live-waiting")
            .font(k.ty.small)
            .color(p.faint()),
        ),
    };
    let (words, action) = match driver {
        Driver::Finished => (tr("This task has finished."), None),
        Driver::Person => {
            let (id, hand_back) = (task.to_owned(), actions.hand_back);
            (
                tr("The window is yours. Hand back to move it out of your way again."),
                Some(
                    Button::new(tr("Hand back"))
                        .variant(ButtonVariant::Contrast)
                        .on_press(move |s: &mut S| hand_back(s, &id))
                        .build_small_push(k)
                        .key("live-hand-back"),
                ),
            )
        }
        Driver::Agent => {
            let (id, take_over) = (task.to_owned(), actions.take_over);
            (
                tr(
                    "The assistant works in this window off your screen. Take over to bring it to you.",
                ),
                Some(
                    Button::new(tr("Take over"))
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut S| take_over(s, &id))
                        .build_small_push(k)
                        .key("live-take-over"),
                ),
            )
        }
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
    col()
        .key("live-card")
        .role("dialog")
        .label(tr("Live app window"))
        .w(w.max(room.0 * 0.6) + PAD * 2.0)
        .padding(PAD)
        .gap(12.0)
        .radius(k.radius.lg + 4.0)
        .fill(k.c.bg_base)
        .border(p.hair, k.c.border_weak)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 12.0,
            blur: 36.0,
            spread: 0.0,
            color: rune::Color::BLACK.alpha(if k.dark { 0.5 } else { 0.18 }),
            inset: false,
        })
        .hit()
        .on_press(|_: &mut S| {})
        .child(header)
        .child(
            el().key("live-picture-row")
                .w_full()
                .center()
                .child(picture),
        )
        .child(bar)
}

/// A small dot: green while the agent works, the accent while the person
/// drives, grey once finished.
fn live_dot(p: Paint, driver: Driver) -> El {
    let k = p.k;
    let colour = match driver {
        Driver::Agent => k.c.success.icon,
        Driver::Person => k.c.icon_accent,
        Driver::Finished => k.alpha(k.c.text_base, 0.3),
    };
    el().key("live-dot")
        .role("none")
        .size(8.0, 8.0)
        .radius(4.0)
        .fill(colour)
}

/// The live picture at `size`; while the person drives (`driving`), it
/// takes their pointer, wheel, and keys to the page.
fn picture<S: 'static>(
    p: Paint,
    preview: Option<&Preview>,
    task: &str,
    session: &str,
    size: (f32, f32),
    viewport: (f32, f32),
    driving: bool,
) -> El {
    let k = p.k;
    let base = el()
        .key("live-picture")
        .size(size.0, size.1)
        .radius(10.0)
        .fill(k.c.layer_02)
        .border(
            if driving { 2.0 * p.hair } else { p.hair },
            if driving {
                k.c.border_focus
            } else {
                k.alpha(k.c.text_base, 0.14)
            },
        );
    let base = match preview.filter(|preview| preview.has_frame()) {
        Some(preview) => base.pixels_fit(preview.large_pixels(), rune::SurfaceFit::Fit),
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
    if !driving {
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
        .on_drag(move |_: &mut S, drag: &mut rune::Drag, _| {
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
        .on_pointer_move_local(move |_: &mut S, pointer: rune::PointerEvent, tx| {
            let (x, y) = live::to_page((pointer.x, pointer.y), size, viewport);
            live::send(&move_task, &move_session, &Input::Move { x, y });
            tx.live_only();
        })
        .on_wheel(move |_: &mut S, wheel: rune::WheelEvent, _| {
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
        .on_key(move |_: &mut S, key: &rune::KeyEvent, _| {
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
        let room = (880.0, 560.0);
        assert_eq!(fitted((1280.0, 720.0), room), (880.0, 495.0));
        assert_eq!(fitted((800.0, 1200.0), room), (373.33334, 560.0));
        assert_eq!(fitted((0.0, 0.0), room), (880.0, 550.0));
        // The overlay's smaller room.
        assert_eq!(fitted((1280.0, 800.0), (720.0, 450.0)), (720.0, 450.0));
    }
}
