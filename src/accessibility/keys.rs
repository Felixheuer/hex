//! What a key would trigger in another app, read through Accessibility the
//! same way for every app: the menu items that carry a key equivalent (the
//! menu bar's own record of an app's shortcuts), and what Return would act
//! on in the focused window (the default button, and the controls nearest
//! the focused field). The guard classifies their titles (`autonomy`).

use std::ffi::c_void;
use std::time::Instant;

use super::{
    CfObject, CfTypeRef, copy_attribute, copy_element, copy_elements, copy_string,
    foreign_application,
};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFGetTypeID(value: CfTypeRef) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CfTypeRef, number_type: isize, value: *mut c_void) -> u8;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(boolean: CfTypeRef) -> u8;
}

/// `kCFNumberSInt64Type`.
const CF_NUMBER_SINT64: isize = 4;

/// A menu item's key equivalent as Accessibility reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuKey {
    pub title: String,
    /// `AXMenuItemCmdChar` ("D", "\r", …).
    pub cmd_char: Option<String>,
    /// `AXMenuItemCmdVirtualKey`.
    pub virtual_key: Option<i64>,
    /// `AXMenuItemCmdGlyph` (⌫, ↩, … for keys without a character).
    pub glyph: Option<i64>,
    /// `AXMenuItemCmdModifiers`: shift 1, option 2, control 4, no ⌘ 8.
    pub modifiers: i64,
    pub enabled: bool,
}

/// Every menu item of app `pid` that has a key equivalent, until
/// `deadline` (menus of a big app are read in a few hundred milliseconds;
/// what isn't read in time is left out).
pub fn menu_keys(pid: i32, deadline: Instant) -> Vec<MenuKey> {
    const MAX_ITEMS: usize = 3000;
    let Some(application) = foreign_application(pid) else {
        return Vec::new();
    };
    let Some(bar) = copy_element(&application, c"AXMenuBar") else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    // (menu, depth): the menus of the bar's items, then their submenus.
    let mut menus: std::collections::VecDeque<(CfObject, u8)> = copy_elements(&bar, c"AXChildren")
        .iter()
        .flat_map(|item| copy_elements(item, c"AXChildren"))
        .map(|menu| (menu, 0))
        .collect();
    let mut seen = 0;
    while let Some((menu, depth)) = menus.pop_front() {
        for item in copy_elements(&menu, c"AXChildren") {
            seen += 1;
            if seen > MAX_ITEMS || Instant::now() >= deadline {
                return keys;
            }
            if depth < 2 {
                menus.extend(
                    copy_elements(&item, c"AXChildren")
                        .into_iter()
                        .map(|submenu| (submenu, depth + 1)),
                );
            }
            let cmd_char = copy_string(&item, c"AXMenuItemCmdChar").filter(|text| !text.is_empty());
            let virtual_key = copy_number(&item, c"AXMenuItemCmdVirtualKey");
            let glyph = copy_number(&item, c"AXMenuItemCmdGlyph").filter(|glyph| *glyph != 0);
            if cmd_char.is_none() && glyph.is_none() && virtual_key.is_none() {
                continue;
            }
            let Some(title) = copy_string(&item, c"AXTitle").filter(|title| !title.is_empty())
            else {
                continue;
            };
            keys.push(MenuKey {
                title,
                cmd_char,
                virtual_key,
                glyph,
                modifiers: copy_number(&item, c"AXMenuItemCmdModifiers").unwrap_or(0),
                enabled: copy_bool(&item, c"AXEnabled").unwrap_or(true),
            });
        }
    }
    keys
}

/// What Return would act on in app `pid`: its focused element's role, the
/// focused window's default button, and the buttons nearest the focused
/// element (its container's, then the next one out, never the whole
/// window's), nearest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Focused {
    pub role: String,
    pub subrole: String,
    pub default_button: Option<String>,
    pub near: Vec<String>,
}

pub fn focused(pid: i32, deadline: Instant) -> Focused {
    const LEVELS: usize = 3;
    const MAX_ELEMENTS: usize = 400;
    let Some(application) = foreign_application(pid) else {
        return Focused::default();
    };
    let mut found = Focused::default();
    if let Some(window) = copy_element(&application, c"AXFocusedWindow") {
        found.default_button = copy_element(&window, c"AXDefaultButton")
            .as_ref()
            .and_then(label);
    }
    let Some(element) = copy_element(&application, c"AXFocusedUIElement") else {
        return found;
    };
    found.role = copy_string(&element, c"AXRole").unwrap_or_default();
    found.subrole = copy_string(&element, c"AXSubrole").unwrap_or_default();
    let mut container = element;
    for _ in 0..LEVELS {
        let Some(parent) = copy_element(&container, c"AXParent") else {
            break;
        };
        if matches!(
            copy_string(&parent, c"AXRole").as_deref(),
            Some("AXWindow" | "AXApplication" | "AXSheet")
        ) {
            break;
        }
        let mut queue: std::collections::VecDeque<CfObject> =
            copy_elements(&parent, c"AXChildren").into();
        let mut seen = 0;
        while let Some(child) = queue.pop_front() {
            seen += 1;
            if seen > MAX_ELEMENTS || Instant::now() >= deadline {
                return found;
            }
            let role = copy_string(&child, c"AXRole").unwrap_or_default();
            if role.ends_with("Button") {
                if let Some(label) = label(&child)
                    && !found.near.contains(&label)
                {
                    found.near.push(label);
                }
            } else {
                queue.extend(copy_elements(&child, c"AXChildren"));
            }
        }
        container = parent;
    }
    found
}

/// A control's label: its title, else its description.
fn label(element: &CfObject) -> Option<String> {
    [c"AXTitle", c"AXDescription"]
        .into_iter()
        .find_map(|name| copy_string(element, name).filter(|text| !text.trim().is_empty()))
}

fn copy_number(element: &CfObject, name: &std::ffi::CStr) -> Option<i64> {
    let value = copy_attribute(element, name)?;
    // SAFETY: `value` is a live CF object; the type is checked first.
    if unsafe { CFGetTypeID(value.0) } != unsafe { CFNumberGetTypeID() } {
        return None;
    }
    let mut number = 0_i64;
    // SAFETY: A CFNumber, read as a 64-bit integer into `number`.
    let read =
        unsafe { CFNumberGetValue(value.0, CF_NUMBER_SINT64, (&raw mut number).cast()) } != 0;
    read.then_some(number)
}

fn copy_bool(element: &CfObject, name: &std::ffi::CStr) -> Option<bool> {
    let value = copy_attribute(element, name)?;
    // SAFETY: `value` is a live CF object; the type is checked first.
    if unsafe { CFGetTypeID(value.0) } != unsafe { CFBooleanGetTypeID() } {
        return None;
    }
    // SAFETY: A CFBoolean.
    Some(unsafe { CFBooleanGetValue(value.0) } != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_missing_apps_have_no_menus_or_focus() {
        let soon = Instant::now() + std::time::Duration::from_millis(50);
        assert!(menu_keys(std::process::id() as i32, soon).is_empty());
        assert!(menu_keys(i32::MAX, soon).is_empty());
        assert_eq!(focused(std::process::id() as i32, soon), Focused::default());
        assert_eq!(focused(i32::MAX, soon), Focused::default());
    }
}
