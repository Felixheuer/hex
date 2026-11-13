//! The permission rows' state: whether macOS reports each grant, whether
//! it waits for a restart, and asking for one (System Settings with Agent-Desk's
//! floating guide). Cua Driver acts with Agent-Desk's grants.

use super::*;

/// Whether macOS reports `grant` on; `None` until it was read.
pub(super) fn granted(ui: &Ui, flow: &SetupFlow, grant: Grant) -> Option<bool> {
    let hex = |state: PermissionState| Some(state == PermissionState::Ready);
    let shared = |hex: Option<bool>, accessibility: bool| {
        if ui.platform.caps.os == super::super::super::platform::Os::Macos
            && let Some(desktop) = flow.desktop.as_ref().filter(|found| found.path.is_some())
        {
            hex.map(|ready| {
                ready
                    && if accessibility {
                        desktop.accessibility
                    } else {
                        desktop.screen_recording
                    }
            })
        } else {
            hex
        }
    };
    match grant {
        Grant::Microphone => hex(ui.platform.permissions.microphone),
        Grant::Accessibility => shared(hex(ui.platform.permissions.accessibility), true),
        Grant::ScreenRecording => shared(flow.permissions.map(|read| read.screen_recording), false),
        Grant::DesktopAccessibility => flow
            .desktop
            .as_ref()
            .filter(|found| found.path.is_some())
            .map(|found| found.accessibility),
        Grant::DesktopScreenRecording => flow
            .desktop
            .as_ref()
            .filter(|found| found.path.is_some())
            .map(|found| found.screen_recording),
    }
}

/// How `grant`'s row stands.
pub(in crate::rune_ui) fn grant_state(ui: &Ui, flow: &SetupFlow, grant: Grant) -> State {
    match granted(ui, flow, grant) {
        Some(true) => State::Passed,
        _ if flow.asked.contains(&grant) => State::Waiting,
        Some(false) => State::Failed,
        None if flow.permissions_checking || flow.desktop_checking => State::Checking,
        None => State::Pending,
    }
}

/// Whether macOS wants Agent-Desk restarted before `grant` applies.
pub(in crate::rune_ui) fn needs_restart(ui: &Ui, grant: Grant) -> bool {
    let state = match grant {
        Grant::Microphone => ui.platform.permissions.microphone,
        Grant::Accessibility | Grant::DesktopAccessibility => ui.platform.permissions.accessibility,
        Grant::ScreenRecording | Grant::DesktopScreenRecording => {
            #[cfg(hex_capture)]
            return ui
                .platform
                .scrawl
                .as_ref()
                .is_some_and(|view| view.screen_needs_restart);
            #[cfg(not(hex_capture))]
            return false;
        }
    };
    state == PermissionState::NeedsRestart
}

/// Asks for a permission: Agent-Desk's open their System Settings page with
/// Agent-Desk's floating guide (Cua Driver's are Agent-Desk's). The row waits
/// until macOS reports it on.
pub(in crate::rune_ui) fn allow(ui: &mut Ui, grant: Grant) {
    if let Some(flow) = ui.chats.setup.as_mut()
        && !flow.asked.contains(&grant)
    {
        flow.asked.push(grant);
    }
    if needs_restart(ui, grant) {
        // One click: Agent-Desk relaunches itself and reopens on this page.
        resume::before_restart(ui);
        ui.ask(PlatformRequest::Permission(PermissionAction::Restart));
        return;
    }
    if let Some(kind) = permissions::kind(grant)
        && ui.platform.stale(kind)
    {
        ui.ask(PlatformRequest::Permission(
            PermissionAction::ResetAndPrompt(kind),
        ));
        recheck(ui);
        return;
    }
    match grant {
        Grant::Microphone => {
            let action = match ui.platform.permissions.microphone {
                PermissionState::NeedsRequest => PermissionAction::RequestMicrophone,
                _ => PermissionAction::ShowGuide(PermissionKind::Microphone),
            };
            ui.ask(PlatformRequest::Permission(action));
        }
        Grant::Accessibility => ui.ask(PlatformRequest::Permission(PermissionAction::ShowGuide(
            PermissionKind::Accessibility,
        ))),
        Grant::ScreenRecording | Grant::DesktopScreenRecording
            if ui.platform.caps.os == super::super::super::platform::Os::Macos =>
        {
            ui.ask(PlatformRequest::Permission(PermissionAction::ShowGuide(
                PermissionKind::ScreenRecording,
            )));
        }
        Grant::DesktopAccessibility
            if ui.platform.caps.os == super::super::super::platform::Os::Macos =>
        {
            ui.ask(PlatformRequest::Permission(PermissionAction::ShowGuide(
                PermissionKind::Accessibility,
            )));
        }
        Grant::ScreenRecording => {
            ui.chats
                .jobs
                .run(Job::Fix(Fix::OpenSettings(Permission::ScreenRecording)));
        }
        Grant::DesktopAccessibility => {
            ui.chats
                .jobs
                .run(Job::Fix(Fix::OpenSettings(Permission::Accessibility)));
        }
        Grant::DesktopScreenRecording => {
            ui.chats
                .jobs
                .run(Job::Fix(Fix::OpenSettings(Permission::ScreenRecording)));
        }
    }
    recheck(ui);
}
