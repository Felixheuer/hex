//! The onboarding's Cua Driver page: the desktop driver (installed with one
//! click, kept current, with its version and update status), which display
//! agents work on, and the live self-tests, run on arrival, proving the
//! chosen agent answers and can use the desktop tools. A failed check
//! always shows its fix, even while later checks still run.

use rune::{Align, El, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant};

use super::super::super::controls::{Paint, PushSize};
use super::super::super::lists::small_button;
use super::super::super::onboarding_kit::arrives;
use super::super::Ui;
use super::super::setup::{self, Grant, Page, SetupFlow, grant_state, page_done};
use super::{
    COL_W, TOOL, actions, done_pill, lead, page, push, rows_card, section_label, spinner,
    status_row,
};
use crate::assistant::backends::Fix;
use crate::desktop::i18n::{tr, tr_fill};

/// An install that failed: the end of what it printed, and its command to
/// run in Terminal instead.
pub(in crate::rune_ui) fn install_failure(p: Paint, error: &str, command: Option<&str>) -> El {
    let tail: Vec<&str> = error.lines().rev().take(4).collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    col()
        .key("install-failure")
        .role("status")
        .label(tr("The install didn't finish"))
        .description(tail.clone())
        .w(COL_W)
        .gap(8.0)
        .items_center()
        .transition(arrives())
        .child(
            text(tail)
                .key("tail")
                .font(p.k.ty.small)
                .color(p.danger())
                .align(Align::Center)
                .wrap()
                .max_w(COL_W),
        )
        .children(command.map(|command| {
            let copied = command.to_owned();
            row()
                .key("fallback")
                .gap(8.0)
                .center_y()
                .child(
                    text(command.to_owned())
                        .key("command")
                        .font(p.k.ty.mono)
                        .color(p.muted())
                        .shrink()
                        .truncate(),
                )
                .child(small_button(
                    p,
                    "copy-command",
                    tr("Copy command"),
                    move |_: &mut Ui, tx: &mut Tx| tx.copy(copied.clone()),
                ))
        }))
}

/// Cua Driver's row: installed with its version and Update, or Install.
fn driver_row(flow: &SetupFlow, p: Paint) -> El {
    let found = flow.desktop.as_ref();
    let installed = found.is_some_and(|found| found.path.is_some());
    let (detail, control) = match found {
        _ if flow.fixing == Some(Fix::InstallDriver) => {
            super::browser::installing(p, flow, tr("Installing… this takes a minute"))
        }
        _ if flow.fixing == Some(Fix::UpdateDriver) => {
            super::browser::installing(p, flow, tr("Looking for a newer Cua Driver…"))
        }
        _ if !cfg!(target_os = "macos") => (
            tr("Cua Driver runs on macOS only").to_owned(),
            row().key("no-driver"),
        ),
        None => (String::new(), spinner(p, "reading")),
        // "Cua Driver 0.49.3 · up to date", "… · update failed (kept 0.49.1)".
        Some(found) if found.path.is_some() => (
            if found.update.is_empty() {
                found
                    .version
                    .as_ref()
                    .map(|version| tr_fill("Version {}", version))
                    .unwrap_or_default()
            } else {
                found.update.clone()
            },
            row()
                .key("driver-controls")
                .gap(8.0)
                .center_y()
                .child(done_pill(p, "installed", tr("Installed")))
                .child(push(p, "update", tr("Update"), false, |ui: &mut Ui| {
                    setup::apply(ui, Fix::UpdateDriver)
                })),
        ),
        Some(_) => (
            tr("Operates apps through Accessibility, without your pointer").to_owned(),
            push(p, "install", tr("Install"), true, |ui: &mut Ui| {
                setup::apply(ui, Fix::InstallDriver)
            }),
        ),
    };
    status_row(
        p,
        "cua-driver",
        TOOL,
        installed,
        crate::assistant::driver::NAME,
        &detail,
        control,
    )
}

/// Cua Driver, then the checks proving it works through the agent.
pub(super) fn desktop(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let installed = flow
        .desktop
        .as_ref()
        .is_some_and(|found| found.path.is_some());
    let shared = ui.platform.caps.os == super::super::super::platform::Os::Macos;
    let mut rows: Vec<El> = vec![driver_row(flow, p)];
    if installed && !shared {
        let first_missing = Grant::desktop()
            .iter()
            .find(|grant| grant_state(ui, flow, **grant) != setup::State::Passed)
            .copied();
        rows.extend(Grant::desktop().iter().map(|grant| {
            super::permissions::grant_row(ui, flow, p, *grant, Some(*grant) == first_missing)
        }));
    }
    // With two or more displays: which one agents work on (optional).
    rows.extend(super::super::displays::onboarding_row(ui, p));
    let checks = super::checks::rows(ui, flow, p);
    let failure = flow
        .fix_error
        .as_ref()
        .map(|error| install_failure(p, error, flow.fix_command.as_deref()));
    let done = page_done(ui, flow, Page::Desktop);
    let again = (!flow.checks_running && !done && !flow.checks.is_empty()).then(|| {
        Button::new(tr("Run again"))
            .variant(ButtonVariant::Neutral)
            .on_press(setup::run_checks)
            .build_push(p.k)
            .key("run-again")
    });
    let cards = col()
        .key("desktop-cards")
        .gap(16.0)
        .child(rows_card(p, "desktop-card", rows))
        .children((!checks.is_empty()).then(|| {
            col()
                .key("checks")
                .gap(8.0)
                .child(section_label(p, "checks-label", tr("Making sure it works")))
                .child(rows_card(p, "checks-card", checks))
        }));
    let (title, explained) = if crate::assistant::computer::asks_permissions() {
        (
            tr("Let it use your Mac"),
            tr(
                "Cua Driver clicks and types in apps for the assistant, with Agent-Desk's permissions. Each check sends a real request, so what passes here works for you.",
            ),
        )
    } else {
        (
            tr("Check the assistant"),
            tr(
                "The assistant operates apps through Cua Driver, which runs on macOS only, so here it answers without using apps.",
            ),
        )
    };
    page(
        p,
        "desktop",
        title,
        lead(p, explained),
        Some(cards),
        failure,
        actions(p, done, true).children(again),
    )
}
