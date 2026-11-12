//! The assistant's onboarding on Windows and Linux: the permission page
//! asks for what each OS has, read the way that OS reports it. (Chats shows
//! only on macOS until the runtime is wired elsewhere, so these drive the
//! flow's state rather than its rendered window.)

use super::super::tests::scene;
use super::{Grant, Page, State, grant_state, page_done};
use crate::assistant::backends::{AgentDesktop, Check, Fix};
use crate::assistant::computer::{Permission, Permissions};
use crate::rune_ui::Ui;
use crate::rune_ui::platform::{Capabilities, Os};

fn desktop_page(os: Capabilities) -> rune::Runtime<Ui> {
    let mut rt = scene("assistant-setup-desktop");
    rt.state.platform.caps = os;
    rt
}

#[test]
fn windows_asks_for_no_permissions_only_for_agent_desktop() {
    let rt = desktop_page(Capabilities::WINDOWS);
    assert!(Grant::hex(Os::Windows).is_empty() && Grant::desktop(Os::Windows).is_empty());
    let flow = rt.state.chats.setup.as_ref().unwrap();
    assert!(
        page_done(&rt.state, flow, Page::Desktop),
        "agent-desktop is installed, and nothing else is asked"
    );
    assert!(!super::needs_restart(&rt.state, Grant::Accessibility));
}

#[test]
fn linux_asks_for_the_accessibility_bus_and_the_screenshot_portal() {
    let mut rt = desktop_page(Capabilities::LINUX);
    assert_eq!(
        Grant::hex(Os::Linux),
        [Grant::Accessibility, Grant::ScreenRecording]
    );
    assert_eq!(Grant::desktop(Os::Linux), [Grant::DesktopAccessibility]);
    let flow = rt.state.chats.setup.as_mut().unwrap();
    flow.permissions = Some(Permissions {
        accessibility: false,
        screen_recording: true,
    });
    flow.desktop = Some(AgentDesktop {
        path: Some("/usr/local/bin/agent-desktop".into()),
        version: Some("1.2.0".into()),
        accessibility: true,
        screen_recording: false,
    });
    let flow = rt.state.chats.setup.as_ref().unwrap();
    // Hex's own reading of the bus decides, not the voice setup's.
    assert_eq!(
        grant_state(&rt.state, flow, Grant::Accessibility),
        State::Failed
    );
    assert_eq!(
        grant_state(&rt.state, flow, Grant::DesktopAccessibility),
        State::Passed
    );
    assert!(!page_done(&rt.state, flow, Page::Desktop));
    // Allow opens the settings `computer::open_settings` knows for Linux.
    super::allow(&mut rt.state, Grant::Accessibility);
    assert!(rt.state.chats.jobs.asked.iter().any(|job| matches!(
        job,
        super::super::Job::Fix(Fix::OpenSettings(Permission::Accessibility))
    )));
    let flow = rt.state.chats.setup.as_mut().unwrap();
    flow.permissions = Some(Permissions {
        accessibility: true,
        screen_recording: true,
    });
    let flow = rt.state.chats.setup.as_ref().unwrap();
    assert!(page_done(&rt.state, flow, Page::Desktop));
}

#[test]
fn checks_name_each_oss_own_permissions() {
    assert_eq!(
        super::check_title(Check::DesktopPermissions, Os::Windows),
        "agent-desktop reaches UI Automation"
    );
    assert_eq!(
        super::check_title(Check::DesktopPermissions, Os::Linux),
        "agent-desktop reaches the accessibility bus"
    );
    assert_eq!(
        super::fix_label(Fix::OpenSettings(Permission::Accessibility), Os::Linux),
        "Open Settings…"
    );
    assert_eq!(
        super::fix_label(Fix::OpenSettings(Permission::Accessibility), Os::Macos),
        "Open Accessibility…"
    );
}

#[test]
fn the_checks_on_windows_leave_out_hexs_permissions_and_lightpanda() {
    let mut rt = desktop_page(Capabilities::WINDOWS);
    rt.state.settings.assistant.background_browser = true;
    rt.state.chats.setup.as_mut().unwrap().checks_running = false;
    super::run_checks(&mut rt.state);
    let flow = rt.state.chats.setup.as_ref().unwrap();
    assert!(!flow.checks.is_empty());
    assert!(
        flow.checks
            .iter()
            .all(|result| !matches!(result.check, Check::HexPermissions | Check::LightBrowser))
    );
}
