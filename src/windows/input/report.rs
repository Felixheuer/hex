//! What the hooks tell Annotate: mouse buttons, the pointer, and the
//! modifiers held, read off the hook threads.

use super::*;

pub(super) fn mouse_message(message: u32, data: u32) -> Option<(u32, bool)> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP,
        WM_XBUTTONDOWN, WM_XBUTTONUP,
    };
    let (button, down) = match message {
        WM_LBUTTONDOWN => (0x01, true),
        WM_LBUTTONUP => (0x01, false),
        WM_RBUTTONDOWN => (0x02, true),
        WM_RBUTTONUP => (0x02, false),
        WM_MBUTTONDOWN => (0x04, true),
        WM_MBUTTONUP => (0x04, false),
        WM_XBUTTONDOWN | WM_XBUTTONUP => (
            match data >> 16 {
                1 => 0x05,
                2 => 0x06,
                _ => return None,
            },
            message == WM_XBUTTONDOWN,
        ),
        _ => return None,
    };
    Some((button, down))
}

pub(super) unsafe extern "system" fn mouse_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: Windows supplies a live MSLLHOOKSTRUCT for this callback.
        let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if crate::capture_host::wants_input() {
            report_pointer(wparam as u32, event);
        }
        if let Some((vk, down)) = mouse_message(wparam as u32, event.mouseData)
            && process_button(vk, down, event.time) != Disposition::Pass
        {
            return 1;
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// What a recording Annotate capture takes from the mouse hook: every move,
/// click, and wheel turn, in physical screen pixels as the engine's Windows
/// target frames are. The event still reaches the app beneath.
pub(super) fn report_pointer(message: u32, event: &MSLLHOOKSTRUCT) {
    use crate::capture_host::{Tapped, report_input};
    use scrawl_capture::hosted_input::HostInput;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        LLMHF_INJECTED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
        WM_RBUTTONDOWN, WM_RBUTTONUP,
    };
    if event.flags & LLMHF_INJECTED != 0 {
        return;
    }
    let (x, y) = (f64::from(event.pt.x), f64::from(event.pt.y));
    let modifiers = held_modifiers();
    // One notch is 120 in the high word, signed; up and right are positive.
    let notches = f64::from((event.mouseData >> 16) as i16) / 120.0;
    let input = match message {
        WM_MOUSEMOVE => HostInput::Moved { x, y, modifiers },
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP => HostInput::Button {
            down: matches!(message, WM_LBUTTONDOWN | WM_RBUTTONDOWN),
            right: matches!(message, WM_RBUTTONDOWN | WM_RBUTTONUP),
            x,
            y,
            modifiers,
        },
        WM_MOUSEWHEEL => HostInput::Scroll {
            x,
            y,
            dx: 0.0,
            dy: notches,
        },
        WM_MOUSEHWHEEL => HostInput::Scroll {
            x,
            y,
            dx: notches,
            dy: 0.0,
        },
        _ => return,
    };
    report_input(CaptureInstant::now(), Tapped::Pointer(input));
}

/// A modifier changed while an Annotate capture records: Shift alone inks.
/// `held` includes the change, which the async key state does not yet.
pub(super) fn report_modifiers(held: &ModifierState) {
    use crate::capture_host::{Tapped, report_input};
    use scrawl_capture::hosted_input::HostInput;
    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    // SAFETY: `point` is a valid POINT to fill.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) };
    report_input(
        CaptureInstant::now(),
        Tapped::Pointer(HostInput::ModifiersChanged {
            x: f64::from(point.x),
            y: f64::from(point.y),
            modifiers: scrawl_capture::hosted_input::Modifiers {
                command: held.left_windows || held.right_windows,
                control: held.left_control || held.right_control,
                option: held.left_alt || held.right_alt,
                shift: held.left_shift || held.right_shift,
            },
        }),
    );
}

/// The modifiers held now, as the engine names them: the Windows key is its
/// Command, Alt its Option.
pub(super) fn held_modifiers() -> scrawl_capture::hosted_input::Modifiers {
    scrawl_capture::hosted_input::Modifiers {
        command: key_down(VK_LWIN) || key_down(VK_RWIN),
        control: key_down(VK_CONTROL),
        option: key_down(VK_MENU),
        shift: key_down(VK_SHIFT),
    }
}

// Both low-level hooks feed exactly the same capture and suppression rules.
