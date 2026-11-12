//! What Hex reads through UI Automation on Windows: the focused element's
//! selected text (TextPattern), never the clipboard, and where password
//! fields are. Each read runs on a helper thread of its own and is given up
//! after its budget, so a hung app's provider costs at most that; while one
//! is still blocked, the next read is skipped rather than piling threads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern, TreeScope_Descendants,
    UIA_IsPasswordPropertyId, UIA_TextPatternId,
};

use crate::assistant::computer::privacy::Rect;

/// The most a selection is read, in UTF-16 units (clipped further later).
const MAX_SELECTION: i32 = 40_000;

/// Whether a read is still running (or blocked) on its helper thread.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Runs `read` with a UI Automation client on a helper thread; `None` past
/// `budget`, when it failed, or when an earlier read is still blocked.
fn bounded<T: Send + 'static>(
    budget: Duration,
    read: impl FnOnce(&IUIAutomation) -> windows::core::Result<T> + Send + 'static,
) -> Option<T> {
    if BUSY.swap(true, Ordering::AcqRel) {
        tracing::debug!("skipping a UI Automation read; the last one is still blocked");
        return None;
    }
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("hex-assistant-uia".into())
        .spawn(move || {
            // SAFETY: This thread's own apartment, uninitialized again
            // below, before the thread ends.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let result = (|| {
                // SAFETY: A plain COM activation on this initialized thread.
                let automation: IUIAutomation =
                    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
                read(&automation)
            })();
            if com.is_ok() {
                // SAFETY: Balances the CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
            BUSY.store(false, Ordering::Release);
            let _ = sender.send(result.ok());
        });
    if spawned.is_err() {
        BUSY.store(false, Ordering::Release);
        return None;
    }
    receiver.recv_timeout(budget).ok().flatten()
}

/// The text selected in the focused element, if it shows text and some is
/// selected; never a password field's.
pub fn selected_text(budget: Duration) -> Option<String> {
    bounded(budget, |automation| {
        // SAFETY: COM calls on a live client, on its own thread.
        unsafe {
            let focused = automation.GetFocusedElement()?;
            if focused.CurrentIsPassword()?.as_bool() {
                return Ok(None);
            }
            let pattern =
                focused.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)?;
            let ranges = pattern.GetSelection()?;
            let mut text = String::new();
            for index in 0..ranges.Length()? {
                let range = ranges.GetElement(index)?;
                text.push_str(&range.GetText(MAX_SELECTION)?.to_string());
            }
            Ok(Some(text).filter(|text| !text.trim().is_empty()))
        }
    })
    .flatten()
}

/// The password fields in `windows` (window handles), in global pixels.
pub fn password_fields(windows: Vec<isize>, budget: Duration) -> Vec<Rect> {
    bounded(budget, move |automation| {
        // SAFETY: COM calls on a live client, on its own thread; the
        // handles are only passed to UI Automation, which checks them.
        unsafe {
            let condition = automation
                .CreatePropertyCondition(UIA_IsPasswordPropertyId, &VARIANT::from(true))?;
            let mut rects = Vec::new();
            for window in windows {
                let Ok(root) = automation.ElementFromHandle(HWND(window as *mut _)) else {
                    continue;
                };
                let Ok(found) = root.FindAll(TreeScope_Descendants, &condition) else {
                    continue;
                };
                for index in 0..found.Length().unwrap_or(0) {
                    let Ok(rect) = found
                        .GetElement(index)
                        .and_then(|field| field.CurrentBoundingRectangle())
                    else {
                        continue;
                    };
                    rects.push([
                        f64::from(rect.left),
                        f64::from(rect.top),
                        f64::from(rect.right - rect.left),
                        f64::from(rect.bottom - rect.top),
                    ]);
                }
            }
            Ok(rects)
        }
    })
    .unwrap_or_default()
}
