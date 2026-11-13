//! The onboarding's try-it page: the Action shortcut, tried for real and
//! checked when it works. A hold is the one way to ask: a quick question
//! about the window in front is answered at once, and a bigger job goes to
//! the background on the assistant's own decision. Double-tap opens the
//! task list (optional: only the HUD sees it). Stop all's keys close it.

use rune::{Align, El, col, el, icon, row, text};

use super::super::super::controls::Paint;
use super::super::super::onboarding_kit::{LEAD, keys};
use super::super::Ui;
use super::super::setup::SetupFlow;
use super::super::setup::tutorial::{Gesture, Tutorial};
use super::{
    COL_W, MIC, ROW_H, SPARKLES, STACK, action_keys, actions, done_pill, page, push, rows_card,
    spinner, waiting_pill,
};
use crate::desktop::i18n::{tr, tr_fill};

/// One gesture's row: what to do with the keys, what happened, and a pill.
#[allow(clippy::too_many_arguments)]
fn gesture_row(
    ui: &Ui,
    p: Paint,
    key: &str,
    glyph: &'static rune::Icon,
    verb: &str,
    rest: &str,
    detail: &str,
    ink: rune::Color,
    control: El,
    current: bool,
) -> El {
    let title = format!("{verb} … {rest}");
    let mut gesture = row()
        .key(key.to_owned())
        .role("group")
        .label(title)
        .description(detail.to_owned())
        .min_h(ROW_H)
        .py(10.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(
            icon(glyph, 16.0)
                .key("icon")
                .color(if current { p.accent() } else { p.k.c.icon_base })
                .a11y_hidden(),
        )
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .w(0.0)
                .gap(4.0)
                .child(
                    row()
                        .key("title-row")
                        .gap(6.0)
                        .center_y()
                        .flex_wrap()
                        .a11y_hidden()
                        .child(
                            text(verb.to_owned())
                                .key("verb")
                                .font(p.k.ty.ui_medium)
                                .color(p.text()),
                        )
                        .children(action_keys(ui, p, "keys", false))
                        .child(
                            text(rest.to_owned())
                                .key("rest")
                                .font(p.k.ty.ui_medium)
                                .color(p.text()),
                        ),
                )
                .child(
                    text(detail.to_owned())
                        .key("detail")
                        .font(p.k.ty.small)
                        .color(ink)
                        .wrap()
                        .a11y_hidden(),
                ),
        )
        .child(control);
    if current {
        gesture = gesture.fill(p.k.c.layer_01);
    }
    gesture
}

/// Hold: a question about the window in front, heard, then answered.
fn here_row(ui: &Ui, p: Paint, tutorial: &Tutorial, current: bool) -> El {
    let here = &tutorial.here;
    let (detail, ink, control) = match (&here.heard, &here.answer) {
        (_, Some(Ok(answer))) => (
            format!("“{}” {}", here.heard.clone().unwrap_or_default(), answer),
            p.muted(),
            done_pill(p, "answered", tr("Answered")),
        ),
        (_, Some(Err(why))) => (
            why.clone(),
            p.danger(),
            push(p, "try-again", tr("Try again"), false, |ui: &mut Ui| {
                if let Some(flow) = ui.chats.setup.as_mut() {
                    flow.tutorial.retry_here();
                }
            }),
        ),
        (Some(heard), None) => (
            tr_fill("“{}” Thinking…", heard),
            p.muted(),
            spinner(p, "answering"),
        ),
        (None, None) => (
            tr("For example “What's on this page?” It answers aloud").to_owned(),
            p.muted(),
            if current {
                waiting_pill(p, "listening", tr("Listening"))
            } else {
                el().key("none")
            },
        ),
    };
    gesture_row(
        ui,
        p,
        "gesture-here",
        &MIC,
        tr("Hold"),
        tr("and ask about the window in front"),
        &detail,
        ink,
        control,
        current,
    )
}

/// Hold with a bigger job: the assistant sends it to the background itself;
/// done once a Background task started.
fn background_row(ui: &Ui, p: Paint, tutorial: &Tutorial, current: bool) -> El {
    let (detail, control) = match &tutorial.background {
        Some(asked) => (
            tr_fill("“{}” Running in the background", asked),
            done_pill(p, "started", tr("Started")),
        ),
        None => (
            tr("For example “Research the three best laptops”. It decides on its own to work in the background, with a frame around the screen")
                .to_owned(),
            if current {
                waiting_pill(p, "waiting", tr("Waiting"))
            } else {
                el().key("none")
            },
        ),
    };
    gesture_row(
        ui,
        p,
        "gesture-background",
        &SPARKLES,
        tr("Hold"),
        tr("and give it a bigger job"),
        &detail,
        p.muted(),
        control,
        current,
    )
}

/// Double-tap: the task list, optional.
fn tasks_row(ui: &Ui, p: Paint, tutorial: &Tutorial, current: bool) -> El {
    let (detail, control) = if tutorial.tasks {
        (
            tr("Every task, with Take over and Stop").to_owned(),
            done_pill(p, "opened", tr("Opened")),
        )
    } else {
        (
            tr("Shows every task beside the HUD. Optional").to_owned(),
            if current {
                waiting_pill(p, "waiting", tr("Waiting"))
            } else {
                el().key("none")
            },
        )
    };
    gesture_row(
        ui,
        p,
        "gesture-tasks",
        STACK,
        tr("Double-tap"),
        tr("to see every task"),
        &detail,
        p.muted(),
        control,
        current,
    )
}

/// The three gestures, then Finish.
pub(super) fn try_it(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let tutorial = &flow.tutorial;
    let next = tutorial.next();
    let card = rows_card(
        p,
        "try-card",
        vec![
            here_row(ui, p, tutorial, next == Some(Gesture::Here)),
            background_row(ui, p, tutorial, next == Some(Gesture::Background)),
            tasks_row(ui, p, tutorial, next == Some(Gesture::Tasks)),
        ],
    );
    let stop_all = ui
        .settings
        .assistant
        .stop_all_hotkey
        .as_ref()
        .map(|binding| super::super::super::settings::shortcut_caps(binding, false));
    let stop_hint = row()
        .key("stop-hint")
        .gap(8.0)
        .center_y()
        .children(stop_all.map(|caps| keys(p, "stop-keys", &caps, false)))
        .child(
            text(tr("stops every task at once, from any app"))
                .key("rest")
                .font(p.k.ty.small)
                .color(p.muted()),
        );
    let actions = actions(p, tutorial.done(), true);
    page(
        p,
        "try",
        tr("Try it out"),
        text(tr("Each one is checked as you do it. Try them in any app."))
            .key("lead")
            .font(LEAD)
            .color(p.muted())
            .align(Align::Center)
            .wrap()
            .max_w(COL_W),
        Some(card),
        Some(col().key("notes").gap(10.0).items_center().child(stop_hint)),
        actions,
    )
}
