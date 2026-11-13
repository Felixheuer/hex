//! The flow's scene fixtures: the agents a scene finds, and a page in a
//! given state.

use super::*;

/// The agents a scene finds: Claude Code and Codex ready, OpenCode not
/// running, no API key.
pub(in crate::rune_ui) fn fixture_found() -> Vec<Found> {
    super::super::BACKENDS
        .iter()
        .map(|backend| {
            let (version, ready, note) = match backend {
                AssistantBackend::ClaudeCode => (Some("2.4.1"), true, None),
                AssistantBackend::Codex => (Some("0.91.0"), false, Some("Not signed in")),
                AssistantBackend::OpenCode => (None, false, Some("Not installed")),
                AssistantBackend::ClaudeApi => (None, false, Some("No API key saved")),
            };
            Found {
                backend: *backend,
                path: version.map(|_| format!("/opt/homebrew/bin/{backend:?}").into()),
                version: version.map(str::to_owned),
                ready,
                note: note.map(str::to_owned),
            }
        })
        .collect()
}

/// The flow for scenes, at `page`, with the checks part way.
pub(in crate::rune_ui) fn fixture(ui: &mut Ui, page: Page, all_passed: bool) {
    ui.chats.settings.found = Some(fixture_found());
    ui.platform.permissions.microphone = PermissionState::Ready;
    ui.platform.permissions.accessibility = PermissionState::Ready;
    let backend = ui.settings.assistant.backend;
    let result = |check: Check, state: CheckState, detail: &str, fix: Option<Fix>| CheckResult {
        check,
        state,
        detail: detail.into(),
        fix,
        took: Duration::ZERO,
    };
    let checks = if all_passed {
        vec![
            result(
                Check::HexPermissions,
                CheckState::Ok,
                "Read Finder's window and the selection",
                None,
            ),
            result(
                Check::Answers(backend),
                CheckState::Ok,
                "Replied “OK” in 2.1 s",
                None,
            ),
            result(
                Check::DesktopPermissions,
                CheckState::Ok,
                "Accessibility and Screen Recording are on",
                None,
            ),
            result(
                Check::DesktopRoundTrip,
                CheckState::Ok,
                "Listed 9 apps and 14 windows",
                None,
            ),
            result(
                Check::DesktopThroughAgent(backend),
                CheckState::Ok,
                "Called desktop_status in 6.4 s",
                None,
            ),
        ]
    } else {
        vec![
            result(
                Check::HexPermissions,
                CheckState::Ok,
                "Read Finder's window and the selection",
                None,
            ),
            result(
                Check::Answers(backend),
                CheckState::Ok,
                "Replied “OK” in 2.1 s",
                None,
            ),
            result(
                Check::DesktopPermissions,
                CheckState::Failed,
                "agent-desktop has no Screen Recording",
                Some(Fix::GuidePermissions),
            ),
            result(Check::DesktopRoundTrip, CheckState::Running, "", None),
            result(
                Check::DesktopThroughAgent(backend),
                CheckState::Pending,
                "",
                None,
            ),
        ]
    };
    ui.chats.setup = Some(SetupFlow {
        page,
        later: if all_passed || page.index() <= Page::Desktop.index() {
            Vec::new()
        } else {
            vec![Page::Desktop]
        },
        desktop: Some(AgentDesktop {
            path: Some("/opt/homebrew/bin/agent-desktop".into()),
            version: Some("0.4.2".into()),
            accessibility: true,
            screen_recording: all_passed,
        }),
        permissions: Some(Permissions {
            accessibility: true,
            screen_recording: all_passed,
        }),
        asked: if all_passed {
            Vec::new()
        } else {
            vec![Grant::ScreenRecording]
        },
        checks,
        checks_running: !all_passed,
        checked_backend: Some(backend),
        conflicts: Some(if all_passed {
            Vec::new()
        } else {
            vec![(
                HotkeySlot::Action,
                Conflict {
                    name: "Siri".into(),
                    description: "Press Right Command twice talks to Siri.".into(),
                    taken: true,
                    fix: ConflictFix::SiriSettings,
                },
            )]
        }),
        probes: if all_passed {
            vec![
                (
                    HotkeySlot::Action,
                    ProbeState::Done(Probe::Works(Duration::from_millis(420))),
                ),
                (
                    HotkeySlot::Dictation,
                    ProbeState::Done(Probe::Works(Duration::from_millis(610))),
                ),
            ]
        } else {
            vec![
                (HotkeySlot::Action, ProbeState::Done(Probe::Swallowed)),
                (HotkeySlot::Dictation, ProbeState::Waiting),
            ]
        },
        tutorial: tutorial::Tutorial::fixture(all_passed),
        ..SetupFlow::default()
    });
    if let Some(flow) = ui.chats.setup.as_mut() {
        browser::fixture(flow, backend, all_passed);
    }
}
