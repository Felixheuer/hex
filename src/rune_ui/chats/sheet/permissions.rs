//! The onboarding's one permissions checklist: Accessibility, Screen
//! Recording, the Microphone, Automation (System Events and Finder),
//! Contacts, Reminders and the Calendar, and optional Full Disk Access.
//! Each row has its live state and one button: Allow opens macOS's prompt
//! or its exact System Settings page with Agent-Desk's guide, and the row
//! turns green when macOS reports it on (read every 1.5 s). A grant macOS
//! applies only after a restart gets one button that relaunches Agent-Desk
//! and reopens this page. Back for what's missing after an update, the
//! checklist shows only the rows still missing.

use rune::{El, row, text};
use rune_kit::glyphs;

use super::super::super::controls::Paint;
use super::super::super::onboarding_kit::arrives;
use super::super::Ui;
use super::super::setup::{
    self, Grant, Page, SetupFlow, State, grant_state, needs_restart, page_done,
};
use super::{
    SCREEN, TOOL, actions, done_pill, lead, page, push, rows_card, spinner, status_row,
    waiting_pill,
};
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

/// One grant's row: its glyph (a green check once on), what it's for, and
/// its control.
pub(super) fn grant_row(ui: &Ui, flow: &SetupFlow, p: Paint, grant: Grant, primary: bool) -> El {
    let shared = ui.platform.caps.os == super::super::super::platform::Os::Macos;
    let (glyph, title, detail) = match grant {
        Grant::Microphone => (
            &glyphs::MIC,
            tr("Microphone"),
            tr("Hears you while you hold the shortcut"),
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
    status_row(
        p,
        grant.key(),
        glyph,
        state == State::Passed,
        title,
        detail,
        grant_control(ui, flow, p, grant, primary),
    )
}

fn automation_done() -> bool {
    use crate::desktop::automation::{self, AutomationState};
    automation::last() == AutomationState::Granted
}

/// Automation: Agent-Desk may control other apps (System Events and Finder),
/// asked here so tasks never stop midway for macOS's prompt. Allow shows
/// one prompt each; a denial opens System Settings › Automation. Other apps
/// ask on first use, and the task's step says so first.
fn automation_row(p: Paint, primary: bool) -> El {
    use crate::desktop::automation::{self, AutomationState};
    let control = match automation::last() {
        AutomationState::Granted => done_pill(p, "allowed", tr("Allowed")),
        AutomationState::Denied => push(
            p,
            "automation-settings",
            tr("Open System Settings"),
            primary,
            |_: &mut Ui| automation::open_settings(),
        ),
        AutomationState::NotAsked | AutomationState::Unknown => {
            push(p, "allow", tr("Allow"), primary, |_: &mut Ui| {
                automation::request_async()
            })
        }
    };
    let detail = match automation::last() {
        AutomationState::Denied => {
            tr("Turn on System Events and Finder under Agent-Desk in System Settings › Automation")
        }
        _ => tr("Lets tasks operate your apps through System Events and Finder"),
    };
    status_row(
        p,
        "automation",
        TOOL,
        automation_done(),
        tr("Automation"),
        detail,
        control,
    )
}

fn personal_data_done() -> bool {
    use crate::desktop::data_access;
    data_access::to_ask(data_access::last).is_empty()
        && data_access::refused(data_access::last).is_empty()
}

/// Contacts, Reminders, and the Calendar (`desktop::data_access`), in one
/// row: Allow shows macOS's prompt for each never answered, one after the
/// other. A refusal stays the person's answer; System Settings turns it on.
fn personal_data_row(p: Paint, primary: bool) -> El {
    use crate::desktop::data_access::{self, Source};
    let name = |source: Source| match source {
        Source::Contacts => tr("Contacts"),
        Source::Reminders => tr("Reminders"),
        Source::Calendar => tr("Calendar"),
        Source::FullDisk => tr("Full Disk Access"),
    };
    let to_ask = data_access::to_ask(data_access::last);
    let refused = data_access::refused(data_access::last);
    let what = tr("Finds who you mean, and reads your reminders and agenda, when you ask");
    let (detail, control) = if !to_ask.is_empty() {
        (
            what.to_owned(),
            push(p, "allow", tr("Allow"), primary, |_: &mut Ui| {
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
        (what.to_owned(), done_pill(p, "allowed", tr("Allowed")))
    };
    status_row(
        p,
        "personal-data",
        &rune_theme_opencode::icons::OUTLINE_PAIR,
        personal_data_done(),
        tr("Contacts, Reminders, and Calendar"),
        &detail,
        control,
    )
}

fn full_disk_done() -> bool {
    use crate::desktop::data_access::{self, Access, Source};
    data_access::last(Source::FullDisk) == Access::Granted
}

/// Full Disk Access, optional: no prompt, only System Settings.
fn full_disk_row(p: Paint) -> El {
    use crate::desktop::data_access::{self, Source};
    let control = if full_disk_done() {
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
    status_row(
        p,
        "full-disk",
        &rune_theme_opencode::icons::LOCK,
        full_disk_done(),
        tr("Full Disk Access (optional)"),
        tr("Lets the assistant search your Mail and Messages on this Mac"),
        control,
    )
}

/// The checklist's items, in order, each with whether it's on.
#[derive(Clone, Copy, PartialEq)]
enum Item {
    Grant(Grant),
    Automation,
    PersonalData,
    FullDisk,
}

fn items() -> Vec<Item> {
    let mut items: Vec<Item> = [
        Grant::Accessibility,
        Grant::ScreenRecording,
        Grant::Microphone,
    ]
    .into_iter()
    .filter(|grant| Grant::hex().contains(grant))
    .map(Item::Grant)
    .collect();
    if cfg!(target_os = "macos") {
        items.extend([Item::Automation, Item::PersonalData, Item::FullDisk]);
    }
    items
}

fn done(ui: &Ui, flow: &SetupFlow, item: Item) -> bool {
    match item {
        Item::Grant(grant) => grant_state(ui, flow, grant) == State::Passed,
        Item::Automation => automation_done(),
        Item::PersonalData => personal_data_done(),
        Item::FullDisk => full_disk_done(),
    }
}

/// "3 of 5 allowed": the required items only.
fn tally(ui: &Ui, flow: &SetupFlow, items: &[Item], p: Paint) -> El {
    let required: Vec<Item> = items
        .iter()
        .copied()
        .filter(|item| *item != Item::FullDisk)
        .collect();
    let on = required
        .iter()
        .filter(|item| done(ui, flow, **item))
        .count();
    let all = on == required.len();
    text(tr_named_count(on, required.len()))
        .key("tally")
        .role("status")
        .font(p.k.ty.small_medium)
        .color(if all { p.success() } else { p.muted() })
        .transition(arrives())
}

fn tr_named_count(on: usize, of: usize) -> String {
    crate::desktop::i18n::tr_named(
        "{on} of {of} allowed",
        &[("on", &on.to_string()), ("of", &of.to_string())],
    )
}

/// The checklist page.
pub(super) fn permissions(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let mut shown = items();
    // Back for what's missing: only the rows still to do.
    if flow.repair.is_some() {
        shown.retain(|item| *item != Item::FullDisk && !done(ui, flow, *item));
    }
    let first_missing = shown
        .iter()
        .copied()
        .find(|item| *item != Item::FullDisk && !done(ui, flow, *item));
    let rows: Vec<El> = shown
        .iter()
        .map(|item| {
            let primary = Some(*item) == first_missing;
            match item {
                Item::Grant(grant) => grant_row(ui, flow, p, *grant, primary),
                Item::Automation => automation_row(p, primary),
                Item::PersonalData => personal_data_row(p, primary),
                Item::FullDisk => full_disk_row(p),
            }
        })
        .collect();
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
    let note = Some(
        rune::col()
            .key("below")
            .gap(8.0)
            .items_center()
            .children(flow.repair.is_none().then(|| tally(ui, flow, &items(), p)))
            .children(resumed),
    );
    page(
        p,
        "permissions",
        tr("Allow what it needs"),
        lead(
            p,
            tr(
                "Each button opens the right place in System Settings. The list updates by itself as macOS turns each one on.",
            ),
        ),
        Some(rows_card(p, "permissions-card", rows)),
        note,
        actions(p, page_done(ui, flow, Page::Permissions), true),
    )
}
