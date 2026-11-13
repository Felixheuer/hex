//! The onboarding's last page: every step with how it stands (ready, left
//! for later, optional), a Go to for each one left, and Finish, which waits
//! until the chosen agent answers and can use the desktop tools.

use rune::{Align, El, Tx, row, text};
use rune_kit::{Button, ButtonVariant};

use super::super::super::controls::{Paint, PushSize};
use super::super::super::onboarding_kit::arrives;
use super::super::Ui;
use super::super::setup::{self, Page, SetupFlow, can_finish, page_done};
use super::{
    ACTION_H, CHAT, COL_W, GLOBE, KEY, MIC, SPARKLES, TOOL, done_pill, later_button, lead, page,
    push, rows_card, status_row,
};
use crate::desktop::i18n::{tr, tr_fill};

/// A step's glyph while it's left.
fn glyph(step: Page) -> &'static rune::Icon {
    match step {
        Page::Permissions => KEY,
        Page::Agent => CHAT,
        Page::Desktop => TOOL,
        Page::Browser => &GLOBE,
        Page::Shortcut => &MIC,
        _ => &SPARKLES,
    }
}

/// One step's row: ready, or left with Go to.
fn step_row(ui: &Ui, flow: &SetupFlow, p: Paint, step: Page, primary: bool) -> El {
    let ready = page_done(ui, flow, step);
    let detail = if ready {
        ""
    } else if step.optional() {
        tr("Optional. You can set it up later in Assistant settings")
    } else if flow.later.contains(&step) {
        tr("Left for later")
    } else {
        tr("Not done yet")
    };
    let control = if ready {
        done_pill(p, "ready", tr("Ready"))
    } else {
        push(p, "go", tr("Go to"), primary, move |ui: &mut Ui| {
            setup::go(ui, step)
        })
    };
    status_row(
        p,
        &format!("step-{}", step.name()),
        glyph(step),
        ready,
        step.step_name(),
        detail,
        control,
    )
}

pub(super) fn done(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let steps: Vec<Page> = Page::shown()
        .iter()
        .copied()
        .filter(|step| !matches!(step, Page::Welcome | Page::Done))
        .collect();
    let first_left = steps
        .iter()
        .copied()
        .find(|step| !step.optional() && !page_done(ui, flow, *step));
    let rows = steps
        .iter()
        .map(|step| step_row(ui, flow, p, *step, Some(*step) == first_left))
        .collect();
    let finish_ready = can_finish(ui, flow);
    let finish = Button::new(tr("Finish"))
        .variant(ButtonVariant::Contrast)
        .disabled(!finish_ready)
        .on_press_tx(|ui: &mut Ui, tx: &mut Tx| setup::finish(ui, tx))
        .build_push(p.k)
        .key("finish");
    let finish = if finish_ready {
        finish.default_button()
    } else {
        finish
    };
    let blocked = (!finish_ready).then(|| {
        text(tr_fill(
            "Finish waits until {} answers and can use the desktop tools",
            super::super::backend_name(ui.settings.assistant.backend),
        ))
        .key("finish-note")
        .font(p.k.ty.small)
        .color(p.muted())
        .align(Align::Center)
        .wrap()
        .max_w(COL_W)
        .transition(arrives())
    });
    let actions = row()
        .key("action")
        .h(ACTION_H)
        .gap(12.0)
        .center()
        .transition(arrives())
        .child(later_button(p))
        .child(finish);
    let all = steps
        .iter()
        .all(|step| step.optional() || page_done(ui, flow, *step));
    page(
        p,
        "done",
        if all {
            tr("You're all set")
        } else {
            tr("Almost there")
        },
        lead(
            p,
            tr(
                "Hold the Action shortcut in any app and just say what you need. Everything here stays in Assistant settings.",
            ),
        ),
        Some(rows_card(p, "done-card", rows)),
        blocked,
        actions,
    )
}
