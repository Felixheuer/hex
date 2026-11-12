//! Share the host's permission recovery and refresh only affected self-tests.
use super::{Grant, Page, Ui};
use crate::assistant::backends::{Check, CheckState, Fix};
use crate::assistant::computer::Permissions;
use crate::desktop::onboarding::PermissionKind;

pub(super) type Fingerprint = Option<(Permissions, bool, bool)>;

pub(super) fn fingerprint(ui: &Ui) -> Fingerprint {
    let flow = ui.chats.setup.as_ref()?;
    let desktop = flow.desktop.as_ref()?;
    Some((
        flow.permissions?,
        desktop.accessibility,
        desktop.screen_recording,
    ))
}

pub(in crate::rune_ui) fn kind(grant: Grant) -> Option<PermissionKind> {
    match grant {
        Grant::Microphone => Some(PermissionKind::Microphone),
        Grant::Accessibility | Grant::DesktopAccessibility => Some(PermissionKind::Accessibility),
        Grant::ScreenRecording | Grant::DesktopScreenRecording => {
            Some(PermissionKind::ScreenRecording)
        }
    }
}

pub(in crate::rune_ui) fn desktop_grant(ui: &Ui) -> Grant {
    if ui
        .chats
        .setup
        .as_ref()
        .and_then(|flow| flow.desktop.as_ref())
        .is_some_and(|desktop| desktop.accessibility)
    {
        Grant::DesktopScreenRecording
    } else {
        Grant::DesktopAccessibility
    }
}

pub(in crate::rune_ui) fn stale_fix(ui: &Ui, fix: Fix) -> bool {
    let kind = match fix {
        Fix::OpenSettings(crate::assistant::computer::Permission::Accessibility) => {
            PermissionKind::Accessibility
        }
        Fix::OpenSettings(crate::assistant::computer::Permission::ScreenRecording) => {
            PermissionKind::ScreenRecording
        }
        Fix::GuidePermissions => kind(desktop_grant(ui)).unwrap(),
        _ => return false,
    };
    ui.platform.stale(kind)
}

pub(super) fn received(ui: &mut Ui, previous: Fingerprint) {
    let current = fingerprint(ui);
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if flow.page != Page::Checks {
        return;
    }
    if previous.is_some() && current.is_some() && previous != current {
        flow.permission_checks_pending = true;
    }
    if !flow.permission_checks_pending || flow.checks_running {
        return;
    }
    flow.permission_checks_pending = false;
    let checks: Vec<_> = flow
        .checks
        .iter()
        .filter_map(|result| match result.check {
            Check::HexPermissions | Check::DesktopPermissions | Check::DesktopRoundTrip => {
                Some(result.check)
            }
            Check::DesktopThroughAgent(backend)
                if flow
                    .checks
                    .iter()
                    .any(|result| result.check == Check::Answers(backend) && result.ok()) =>
            {
                Some(result.check)
            }
            _ => None,
        })
        .collect();
    if checks.is_empty() {
        return;
    }
    for result in &mut flow.checks {
        if checks.contains(&result.check) {
            result.state = CheckState::Pending;
            result.detail.clear();
            result.fix = None;
        }
    }
    flow.checks_running = true;
    ui.chats.jobs.run(super::Job::SelfTests(
        Box::new(ui.settings.assistant.clone()),
        checks,
    ));
}
