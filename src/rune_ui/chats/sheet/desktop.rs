//! The onboarding's permissions page: what the assistant needs to see and
//! use the Mac. Screen Recording for Agent-Desk, the Microphone and Accessibility
//! only when the voice setup didn't already grant them (one ask for both),
//! then Automation (System Events and Finder), Cua Driver (installed with
//! one click, kept current, with its version and update status), then the
//! person's data (Contacts,
//! Reminders, the Calendar, and optional Full Disk Access). A grant macOS
//! applies only after a restart gets one button that relaunches Agent-Desk and
//! reopens this page.

use rune::{Align, El, Tx, col, row, text};
use rune_kit::glyphs;

use super::super::super::controls::Paint;
use super::super::super::lists::small_button;
use super::super::super::onboarding_kit::arrives;
use super::super::Ui;
use super::super::setup::{
    self, Grant, Page, SetupFlow, State, grant_state, needs_restart, page_done,
};
use super::{
    COL_W, SCREEN, TOOL, actions, done_pill, lead, page, push, row_of, rows_card, spinner,
    waiting_pill,
};
use crate::assistant::backends::Fix;
use crate::desktop::i18n::{tr, tr_fill};

/// A permission's control: Allow, then a waiting dot until macOS reports
/// it on, then Allowed; Restart Agent-Desk and continue when macOS wants that.
fn grant_control(ui: &Ui, flow: &SetupFlow, p: Paint, grant: Grant, primary: bool) -> El {
    if needs_restart(ui, grant) {
        if !primary {
            return waiting_pill(p, "restart-needed", tr("Restart Agent-Desk and continue"));
        }
        return push(
            p,
            "restart",
            tr("Restart Agent-Desk and continue"),
            true,
            move |ui: &mut Ui| setup::allow(ui, grant),
        );
    }
    if setup::permissions::kind(grant).is_some_and(|kind| ui.platform.stale(kind)) {
        return push(
            p,
            "repair",
            tr("Repair permission entry"),
            primary,
            move |ui: &mut Ui| setup::allow(ui, grant),
        );
    }
    match grant_state(ui, flow, grant) {
        State::Passed => done_pill(p, "allowed", tr("Allowed")),
        State::Checking | State::Pending => spinner(p, "reading"),
        State::Waiting if needs_restart(ui, grant) => push(
            p,
            "restart",
            tr("Restart Agent-Desk and continue"),
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
    let shared = ui.platform.caps.os == super::super::super::platform::Os::Macos;
    let (glyph, title, detail) = match grant {
        Grant::Microphone => (
            &glyphs::MIC,
            tr("Microphone"),
            tr("Hears your question while you hold the shortcut"),
        ),
        Grant::Accessibility => (
            &glyphs::CURSOR_TEXT,
            tr("Accessibility"),
            if shared {
                tr("Lets Agent-Desk and Cua Driver read and operate apps")
            } else {
                tr("Notices the shortcut and reads the text you selected")
            },
        ),
        Grant::ScreenRecording => (
            SCREEN,
            tr("Screen Recording"),
            if shared {
                tr("Lets Agent-Desk and Cua Driver see the screen")
            } else {
                tr("Sends the screen you ask about with your question")
            },
        ),
        Grant::DesktopAccessibility => (
            &glyphs::CURSOR_TEXT,
            tr("Accessibility"),
            tr("Lets Cua Driver click, type, and read apps"),
        ),
        Grant::DesktopScreenRecording => (
            SCREEN,
            tr("Screen Recording"),
            tr("Lets Cua Driver see windows while a task works"),
        ),
    };
    let state = grant_state(ui, flow, grant);
    let detail = match state {
        _ if setup::permissions::kind(grant).is_some_and(|kind| ui.platform.stale(kind)) => tr(
            "Already enabled? Repair the entry for this copy of Agent-Desk, then allow it again.",
        ),
        State::Waiting if needs_restart(ui, grant) => {
            tr("macOS turns it on when Agent-Desk restarts. Agent-Desk reopens right here")
        }
        State::Waiting => tr("Switch on Agent-Desk in System Settings"),
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

/// Automation: Agent-Desk may control other apps (System Events and Finder),
/// asked here so tasks never stop midway for macOS's prompt. Allow shows
/// one prompt each; a denial opens System Settings › Automation. Other apps
/// ask on first use, and the task's step says so first.
fn automation_row(p: Paint) -> El {
    use crate::desktop::automation::{self, AutomationState};
    let control = match automation::last() {
        AutomationState::Granted => done_pill(p, "allowed", tr("Allowed")),
        AutomationState::Denied => push(
            p,
            "automation-settings",
            tr("Open System Settings"),
            true,
            |_: &mut Ui| automation::open_settings(),
        ),
        AutomationState::NotAsked | AutomationState::Unknown => {
            push(p, "allow", tr("Allow"), true, |_: &mut Ui| {
                automation::request_async()
            })
        }
    };
    let detail = match automation::last() {
        AutomationState::Denied => {
            tr("Turn on System Events and Finder under Agent-Desk in System Settings › Automation")
        }
        _ => tr(
            "Lets tasks operate your apps through System Events and Finder. Other apps ask the first time a task uses them",
        ),
    };
    row_of(
        p,
        "automation",
        TOOL,
        tr("Control other apps"),
        detail,
        p.muted(),
        control,
    )
}

/// Contacts, Reminders, and the Calendar (`desktop::data_access`), in one
/// row: Allow shows macOS's prompt for each never answered, one after the
/// other. A refusal stays the person's answer; System Settings turns it on.
fn personal_data_row(p: Paint) -> El {
    use crate::desktop::data_access::{self, Source};
    let name = |source: Source| match source {
        Source::Contacts => tr("Contacts"),
        Source::Reminders => tr("Reminders"),
        Source::Calendar => tr("Calendar"),
        Source::FullDisk => tr("Full Disk Access"),
    };
    let to_ask = data_access::to_ask(data_access::last);
    let refused = data_access::refused(data_access::last);
    let (detail, control) = if !to_ask.is_empty() {
        (
            tr("Finds who you mean, and reads your reminders and agenda, when you ask").to_owned(),
            push(p, "allow", tr("Allow"), true, |_: &mut Ui| {
                data_access::request_all_async()
            }),
        )
    } else if let Some(first) = refused.first().copied() {
        let names: Vec<&str> = refused.iter().map(|source| name(*source)).collect();
        (
            tr_fill(
                "Off for {}: switch on Agent-Desk there in System Settings",
                &names.join(", "),
            ),
            push(
                p,
                "data-settings",
                tr("Open System Settings"),
                false,
                move |_: &mut Ui| data_access::open_settings(first),
            ),
        )
    } else {
        (
            tr("Finds who you mean, and reads your reminders and agenda, when you ask").to_owned(),
            done_pill(p, "allowed", tr("Allowed")),
        )
    };
    row_of(
        p,
        "personal-data",
        &rune_theme_opencode::icons::OUTLINE_PAIR,
        tr("Contacts, Reminders, and Calendar"),
        &detail,
        p.muted(),
        control,
    )
}

/// Full Disk Access, optional: no prompt, only System Settings.
fn full_disk_row(p: Paint) -> El {
    use crate::desktop::data_access::{self, Access, Source};
    let control = if data_access::last(Source::FullDisk) == Access::Granted {
        done_pill(p, "allowed", tr("Allowed"))
    } else {
        push(
            p,
            "full-disk-settings",
            tr("Open System Settings"),
            false,
            |_: &mut Ui| data_access::open_settings(Source::FullDisk),
        )
    };
    row_of(
        p,
        "full-disk",
        &rune_theme_opencode::icons::LOCK,
        tr("Full Disk Access"),
        tr("Optional: lets the assistant search your Mail and Messages on this Mac"),
        p.muted(),
        control,
    )
}

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

/// Screen Recording (and any voice grant still missing), then Cua Driver
/// with its version and update status.
pub(super) fn desktop(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    // The voice setup asked for the Microphone and Accessibility already;
    // they show again only while missing.
    let hex: Vec<Grant> = Grant::hex()
        .iter()
        .copied()
        .filter(|grant| {
            *grant == Grant::ScreenRecording || grant_state(ui, flow, *grant) != State::Passed
        })
        .collect();
    let found = flow.desktop.as_ref();
    let installed = found.is_some_and(|found| found.path.is_some());
    let mut grants: Vec<Grant> = hex.clone();
    let shared = ui.platform.caps.os == super::super::super::platform::Os::Macos;
    if installed && !shared {
        grants.extend(Grant::desktop());
    }
    let first_missing = grants
        .iter()
        .find(|grant| grant_state(ui, flow, **grant) != State::Passed)
        .copied();
    let mut rows: Vec<El> = hex
        .iter()
        .map(|grant| grant_row(ui, flow, p, *grant, Some(*grant) == first_missing))
        .collect();
    if cfg!(target_os = "macos") {
        rows.push(automation_row(p));
    }
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
                .child(push(
                    p,
                    "update",
                    tr("Update Cua Driver"),
                    false,
                    |ui: &mut Ui| setup::apply(ui, Fix::UpdateDriver),
                )),
        ),
        Some(_) => (
            tr("Operates apps through Accessibility, without your pointer").to_owned(),
            push(p, "install", tr("Install"), true, |ui: &mut Ui| {
                setup::apply(ui, Fix::InstallDriver)
            }),
        ),
    };
    rows.push(row_of(
        p,
        "cua-driver",
        TOOL,
        crate::assistant::driver::NAME,
        &detail,
        p.muted(),
        control,
    ));
    if installed && !shared {
        rows.extend(
            Grant::desktop()
                .iter()
                .map(|grant| grant_row(ui, flow, p, *grant, Some(*grant) == first_missing)),
        );
    }
    // What the assistant may read: macOS asks for each once.
    if cfg!(target_os = "macos") {
        rows.push(personal_data_row(p));
        rows.push(full_disk_row(p));
    }
    // With two or more displays: which one agents work on (optional).
    rows.extend(super::super::displays::onboarding_row(ui, p));
    let resumed = flow.resumed.then(|| {
        text(tr(
            "Agent-Desk restarted. Checking again where you left off…",
        ))
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
    let (title, explained) = if crate::assistant::computer::asks_permissions() {
        (
            tr("Let the assistant see and use your Mac"),
            tr(
                "Each one opens its page in System Settings, and this page waits until macOS turns it on. Agent-Desk starts Cua Driver, so macOS asks for Agent-Desk.",
            ),
        )
    } else if cfg!(target_os = "windows") {
        (
            tr("Let the assistant see and use your PC"),
            tr(
                "The assistant operates apps through Cua Driver, which runs on macOS only, so here it answers without using apps.",
            ),
        )
    } else {
        (
            tr("Let the assistant see and use your computer"),
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
        Some(rows_card(p, "desktop-card", rows)),
        below,
        actions(p, page_done(ui, flow, Page::Desktop), true),
    )
}
