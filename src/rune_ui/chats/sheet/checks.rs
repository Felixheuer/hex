//! The onboarding's self-test rows: the agent answers and can use the
//! desktop tools (and agent-browser). A failed check always shows its fix,
//! even while later checks still run.

use rune::{El, col, el, row, text};

use super::super::super::controls::Paint;
use super::super::Ui;

use super::super::setup::{self, SetupFlow, check_title, fix_label};
use super::{mark, push, spinner};
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
    let blocked = detail.starts_with("Needs “");
    let ink = if state == CheckState::Failed && !blocked {
        p.danger()
    } else {
        p.muted()
    };
    let said = match state {
        _ if blocked => tr("Waiting for a previous check").to_owned(),
        CheckState::Pending => tr("Waiting to run").to_owned(),
        CheckState::Running if detail.is_empty() => tr("Checking…").to_owned(),
        CheckState::Ok => tr("Ready").to_owned(),
        CheckState::Failed if doctor(detail).is_some() => doctor(detail).unwrap().to_owned(),
        CheckState::Failed
            if detail.starts_with("Agent-Desk needs Accessibility")
                || detail.starts_with("Cua Driver has no Accessibility") =>
        {
            tr("Switch on Agent-Desk in System Settings").to_owned()
        }
        CheckState::Failed
            if detail.starts_with("Agent-Desk needs Screen Recording")
                || detail.starts_with("Cua Driver has no Screen Recording") =>
        {
            tr("Open Screen Recording…").to_owned()
        }
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
        .child(mark(p, if blocked { CheckState::Pending } else { state }))
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
                        .wrap()
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

/// The permission doctor's findings, translated (`desktop::permission_doctor`).
fn doctor(detail: &str) -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        use crate::desktop::permission_doctor::{
            NEEDS_RESTART, NOT_INSTALLED, OTHER_COPIES, TRANSLOCATED,
        };
        [TRANSLOCATED, NOT_INSTALLED, OTHER_COPIES, NEEDS_RESTART]
            .into_iter()
            .find(|message| *message == detail)
            .map(tr)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = detail;
        None
    }
}

/// A failed check's fix: its button at once, even while later checks run
/// (only Try again waits for them), and a spinner while it works.
fn fix_control(ui: &Ui, flow: &SetupFlow, p: Paint, result: &CheckResult) -> El {
    if result.detail.starts_with("Needs “") {
        return el().key("none");
    }
    match result.fix {
        Some(fix) if result.state == CheckState::Failed && flow.fixing == Some(fix) => {
            spinner(p, "fixing")
        }
        Some(Fix::Retry) if flow.checks_running => el().key("none"),
        Some(fix) if result.state == CheckState::Failed => {
            let label = if setup::permissions::stale_fix(ui, fix) {
                tr("Repair permission entry").to_owned()
            } else {
                fix_label(fix)
            };
            push(p, "fix", &label, false, move |ui: &mut Ui| {
                setup::apply(ui, fix)
            })
        }
        _ => el().key("none"),
    }
}

/// The live self-tests' rows, run on arrival at the Cua Driver page.
pub(super) fn rows(ui: &Ui, flow: &SetupFlow, p: Paint) -> Vec<El> {
    flow.checks
        .iter()
        .enumerate()
        .map(|(index, result)| {
            result_row(
                p,
                format!("check{index}"),
                check_title(result.check),
                result.state,
                &result.detail,
                fix_control(ui, flow, p, result),
            )
        })
        .collect()
}
