//! The onboarding's Checks page: the agent answers and can use
//! agent-desktop (and agent-browser). A failed check always shows its fix,
//! even while later checks still run.

use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant};

use super::super::super::controls::{Paint, PushSize};
use super::super::Ui;

use super::super::setup::{self, Page, SetupFlow, check_title, fix_label, page_done};
use super::{actions, lead, mark, note, page, push, rows_card, spinner};
use crate::assistant::backends::{CheckResult, CheckState, Fix};
use crate::desktop::i18n::tr;

/// One self-test's row: its mark, what it proves over what it found, and
/// its fix when it failed.
pub(super) fn result_row(
    p: Paint,
    key: String,
    title: String,
    state: CheckState,
    detail: &str,
    control: El,
) -> El {
    let ink = if state == CheckState::Failed {
        p.danger()
    } else {
        p.muted()
    };
    let said = match state {
        CheckState::Pending => tr("Waiting to run").to_owned(),
        CheckState::Running if detail.is_empty() => tr("Checking…").to_owned(),
        _ => detail.to_owned(),
    };
    row()
        .key(key)
        .role("group")
        .label(title.clone())
        .description(said.clone())
        .min_h(48.0)
        .py(8.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(mark(p, state))
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .w(0.0)
                .gap(2.0)
                .child(
                    text(title)
                        .key("title")
                        .font(p.k.ty.ui_medium)
                        .color(p.text())
                        .a11y_hidden(),
                )
                .child(
                    text(said)
                        .key("detail")
                        .font(p.k.ty.small)
                        .color(ink)
                        .wrap()
                        .a11y_hidden(),
                ),
        )
        .child(control)
}

/// A failed check's fix: its button at once, even while later checks run
/// (only Try again waits for them), and a spinner while it works.
fn fix_control(flow: &SetupFlow, p: Paint, result: &CheckResult) -> El {
    match result.fix {
        Some(fix) if result.state == CheckState::Failed && flow.fixing == Some(fix) => {
            spinner(p, "fixing")
        }
        Some(Fix::Retry) if flow.checks_running => el().key("none"),
        Some(fix) if result.state == CheckState::Failed => {
            push(p, "fix", &fix_label(fix), false, move |ui: &mut Ui| {
                setup::apply(ui, fix)
            })
        }
        _ => el().key("none"),
    }
}

/// The live self-tests, run on arrival; Run again after a fix.
pub(super) fn checks(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let rows = flow
        .checks
        .iter()
        .enumerate()
        .map(|(index, result)| {
            result_row(
                p,
                format!("check{index}"),
                check_title(result.check),
                result.state,
                &result.detail,
                fix_control(flow, p, result),
            )
        })
        .collect();
    let done = page_done(ui, flow, Page::Checks);
    let again = (!flow.checks_running && !done).then(|| {
        Button::new(tr("Run again"))
            .variant(ButtonVariant::Neutral)
            .on_press(setup::run_checks)
            .build_push(p.k)
            .key("run-again")
    });
    let actions = actions(p, done, true).children(again);
    page(
        p,
        "checks",
        tr("Making sure it works"),
        lead(
            p,
            tr("Each check sends a real request, so what passes here works for you."),
        ),
        Some(rows_card(p, "checks-card", rows)),
        flow.fix_error.as_ref().map(|error| note(p, error)),
        actions,
    )
}

