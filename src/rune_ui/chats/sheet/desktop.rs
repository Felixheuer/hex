//! The onboarding's permissions page: what the assistant needs to see and
//! use the Mac. Screen Recording for Hex, the Microphone and Accessibility
//! only when the voice setup didn't already grant them (one ask for both),
//! then agent-desktop, installed with one click, and its own grants. A
//! grant macOS applies only after a restart gets one button that relaunches
//! Hex and reopens this page.

use rune::{Align, El, Tx, col, row, text};
use rune_kit::glyphs;

use super::super::super::controls::Paint;
use super::super::super::lists::small_button;
use super::super::super::onboarding_kit::arrives;
use super::super::Ui;
use super::super::setup::{self, Grant, Page, SetupFlow, State, grant_state, needs_restart, page_done};
use super::{
    COL_W, SCREEN, TOOL, actions, done_pill, lead, page, push, row_of, rows_card, spinner,
    waiting_pill,
};
use crate::assistant::backends::Fix;
use crate::desktop::i18n::{tr, tr_fill};

/// A permission's control: Allow, then a waiting dot until macOS reports
/// it on, then Allowed; Restart Hex and continue when macOS wants that.
fn grant_control(ui: &Ui, flow: &SetupFlow, p: Paint, grant: Grant, primary: bool) -> El {
    match grant_state(ui, flow, grant) {
        State::Passed => done_pill(p, "allowed", tr("Allowed")),
        State::Checking | State::Pending => spinner(p, "reading"),
        State::Waiting if needs_restart(ui, grant) => push(
            p,
            "restart",
            tr("Restart Hex and continue"),
            true,
            move |ui: &mut Ui| setup::allow(ui, grant),
        ),
        State::Waiting => row()
            .key("waiting-slot")
            .gap(8.0)
            .center_y()
            .child(waiting_pill(p, "waiting", tr("Waiting")))
            .child(push(
                p,
                "again",
                tr("Open again"),
                false,
                move |ui: &mut Ui| setup::allow(ui, grant),
            )),
        State::Failed => push(p, "allow", tr("Allow"), primary, move |ui: &mut Ui| {
            setup::allow(ui, grant)
        }),
    }
}

pub(super) fn grant_row(ui: &Ui, flow: &SetupFlow, p: Paint, grant: Grant, primary: bool) -> El {
    let (glyph, title, detail) = match grant {
        Grant::Microphone => (
            &glyphs::MIC,
            tr("Microphone"),
            tr("Hears your question while you hold the shortcut"),
        ),
        Grant::Accessibility => (
            &glyphs::CURSOR_TEXT,
            tr("Accessibility"),
            tr("Notices the shortcut and reads the text you selected"),
        ),
        Grant::ScreenRecording => (
            SCREEN,
            tr("Screen Recording"),
            tr("Sends the screen you ask about with your question"),
        ),
        Grant::DesktopAccessibility => (
            &glyphs::CURSOR_TEXT,
            tr("Accessibility"),
            tr("Lets agent-desktop click, type, and read apps"),
        ),
        Grant::DesktopScreenRecording => (
            SCREEN,
            tr("Screen Recording"),
            tr("Lets agent-desktop see windows while a task works"),
        ),
    };
    let state = grant_state(ui, flow, grant);
    let detail = match state {
        State::Waiting if needs_restart(ui, grant) => {
            tr("macOS turns it on when Hex restarts. Hex reopens right here")
        }
        State::Waiting => tr("Switch on Hex in System Settings"),
        _ => detail,
    };
    row_of(
        p,
        grant.key(),
        glyph,
        title,
        detail,
        p.muted(),
        grant_control(ui, flow, p, grant, primary),
    )
}

/// An install that failed: the end of what it printed, and its command to
/// run in Terminal instead.
pub(super) fn install_failure(p: Paint, error: &str, command: Option<&str>) -> El {
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

/// Screen Recording (and any voice grant still missing), then agent-desktop
/// and its grants.
pub(super) fn desktop(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    // The voice setup asked for the Microphone and Accessibility already;
    // they show again only while missing.
    let hex: Vec<Grant> = Grant::HEX
        .into_iter()
        .filter(|grant| {
            *grant == Grant::ScreenRecording || grant_state(ui, flow, *grant) != State::Passed
        })
        .collect();
    let found = flow.desktop.as_ref();
    let installed = found.is_some_and(|found| found.path.is_some());
    let mut grants: Vec<Grant> = hex.clone();
    if installed {
        grants.extend(Grant::DESKTOP);
    }
    let first_missing = grants
        .iter()
        .find(|grant| grant_state(ui, flow, **grant) != State::Passed)
        .copied();
    let mut rows: Vec<El> = hex
        .iter()
        .map(|grant| grant_row(ui, flow, p, *grant, Some(*grant) == first_missing))
        .collect();
    let (detail, control) = match found {
        _ if flow.fixing == Some(Fix::InstallAgentDesktop) => {
            super::browser::installing(p, flow, tr("Installing… this takes a minute"))
        }
        None => (String::new(), spinner(p, "reading")),
        Some(found) if found.path.is_some() => (
            found
                .version
                .as_ref()
                .map(|version| tr_fill("Version {}", version))
                .unwrap_or_default(),
            done_pill(p, "installed", tr("Installed")),
        ),
        Some(_) => (
            tr("Operates apps through Accessibility, without your pointer").to_owned(),
            push(p, "install", tr("Install"), true, |ui: &mut Ui| {
                setup::apply(ui, Fix::InstallAgentDesktop)
            }),
        ),
    };
    rows.push(row_of(
        p,
        "agent-desktop",
        TOOL,
        "agent-desktop",
        &detail,
        p.muted(),
        control,
    ));
    if installed {
        rows.extend(
            Grant::DESKTOP
                .iter()
                .map(|grant| grant_row(ui, flow, p, *grant, Some(*grant) == first_missing)),
        );
    }
    let resumed = flow.resumed.then(|| {
        text(tr("Hex restarted. Checking again where you left off…"))
            .key("resumed")
            .role("status")
            .font(p.k.ty.small_medium)
            .color(p.accent())
            .transition(arrives())
    });
    let failure = flow
        .fix_error
        .as_ref()
        .map(|error| install_failure(p, error, flow.fix_command.as_deref()));
    let below = match (resumed, failure) {
        (None, None) => None,
        (resumed, failure) => Some(
            col()
                .key("below")
                .gap(8.0)
                .items_center()
                .children(resumed)
                .children(failure),
        ),
    };
    page(
        p,
        "desktop",
        tr("Let the assistant see and use your Mac"),
        lead(
            p,
            tr("Each one opens its page in System Settings, and this page waits until macOS turns it on. Hex starts agent-desktop, so macOS asks for Hex."),
        ),
        Some(rows_card(p, "desktop-card", rows)),
        below,
        actions(p, page_done(ui, flow, Page::Desktop), true),
    )
}
