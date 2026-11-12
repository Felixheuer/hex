//! Key names, numpad codes, and keycaps: how a binding's key names become
//! the virtual keys the keyboard hook sees, and back.

use super::*;

/// Numpad Enter's code in a binding: Enter's virtual key with the extended
/// flag, which no virtual key of its own names.
pub(super) const NUMPAD_ENTER: u32 = 0x100 | VK_RETURN as u32;

/// The numpad key a keyboard hook last saw go down and not yet up, by its
/// `num` code, for the chord capture: polled key state can't tell Numpad 1
/// with Num Lock off from End. Zero for none. The capture runs a hook of
/// its own (`NumpadWatch`), so it doesn't depend on the listener's.
pub(super) static NUMPAD_DOWN: AtomicU32 = AtomicU32::new(0);

/// The numpad digit and decimal keys by scan code. With Num Lock off they
/// send the navigation keys' codes (Numpad 1 is End), but keep these scan
/// codes without the extended flag, which the navigation cluster's keys
/// carry.
pub(super) const NUMPAD_SCAN_CODES: [(u32, u32); 11] = [
    (0x52, 0x60),
    (0x4F, 0x61),
    (0x50, 0x62),
    (0x51, 0x63),
    (0x4B, 0x64),
    (0x4C, 0x65),
    (0x4D, 0x66),
    (0x47, 0x67),
    (0x48, 0x68),
    (0x49, 0x69),
    (0x53, 0x6E),
];

/// Notes a numpad key going down or up for the chord capture
/// (`NUMPAD_DOWN`), and returns its `num` code; `None` for any other key.
/// Only a hook tells a numpad key from the key it stands for.
pub(super) fn track_numpad(event: &KBDLLHOOKSTRUCT, is_down: bool) -> Option<u32> {
    let vk = numpad_key(event)?;
    if is_down {
        NUMPAD_DOWN.store(vk, Ordering::Relaxed);
    } else {
        let _ = NUMPAD_DOWN.compare_exchange(vk, 0, Ordering::Relaxed, Ordering::Relaxed);
    }
    Some(vk)
}

/// A numpad key's `num` code, whatever Num Lock says: the digits and the
/// decimal by scan code, Numpad Enter by Enter's extended flag, and the
/// operators by their own virtual keys.
pub(super) fn numpad_key(event: &KBDLLHOOKSTRUCT) -> Option<u32> {
    let extended = event.flags & LLKHF_EXTENDED != 0;
    if event.vkCode == u32::from(VK_RETURN) {
        return extended.then_some(NUMPAD_ENTER);
    }
    if matches!(event.vkCode, 0x6A | 0x6B | 0x6D | 0x6F) {
        return Some(event.vkCode);
    }
    if extended {
        return None;
    }
    NUMPAD_SCAN_CODES
        .into_iter()
        .find(|&(scan, _)| scan == event.scanCode)
        .map(|(_, vk)| vk)
}

/// Non-modifier keys the capture scans, paired with the names
/// [`virtual_key`] accepts (letters, digits, function keys, and the
/// layout's punctuation are scanned separately). Numpad keys are named
/// `num 1`, `num *`, ...: their own keys, not the top row's.
pub(super) const CAPTURE_NAMED_KEYS: [(u16, &str); 45] = [
    (VK_SPACE, "space"),
    (VK_RETURN, "enter"),
    (VK_TAB, "tab"),
    (0x08, "backspace"),
    (0x2e, "delete"),
    (0x2d, "insert"),
    (0x24, "home"),
    (0x23, "end"),
    (0x21, "pageup"),
    (0x22, "pagedown"),
    (0x25, "left"),
    (0x26, "up"),
    (0x27, "right"),
    (0x28, "down"),
    (0x13, "pause"),
    (0x91, "scroll lock"),
    (0x5D, "menu"),
    // The punctuation keys, by their layout-independent virtual keys; the
    // keycap shows what each types on the active layout (`keycap_label`).
    (0xBA, "oem 1"),
    (0xBF, "oem 2"),
    (0xC0, "oem 3"),
    (0xDB, "oem 4"),
    (0xDC, "oem 5"),
    (0xDD, "oem 6"),
    (0xDE, "oem 7"),
    (0xDF, "oem 8"),
    (0xBB, "oem plus"),
    (0xBC, "oem comma"),
    (0xBD, "oem minus"),
    (0xBE, "oem period"),
    (0xE2, "oem 102"),
    (0x60, "num 0"),
    (0x61, "num 1"),
    (0x62, "num 2"),
    (0x63, "num 3"),
    (0x64, "num 4"),
    (0x65, "num 5"),
    (0x66, "num 6"),
    (0x67, "num 7"),
    (0x68, "num 8"),
    (0x69, "num 9"),
    (0x6A, "num *"),
    (0x6B, "num +"),
    (0x6D, "num -"),
    (0x6E, "num ."),
    (0x6F, "num /"),
];

/// The keycap for a saved key label: a punctuation key shows the character
/// it types on the active layout now (`;`, `ù`), else its name (`Oem 3`).
pub(crate) fn keycap_label(label: &str) -> String {
    let name = label.to_ascii_lowercase();
    CAPTURE_NAMED_KEYS
        .into_iter()
        .find(|&(_, named)| named == name && named.starts_with("oem "))
        .and_then(|(vk, _)| {
            // SAFETY: a pure lookup in this thread's keyboard layout.
            let mapped = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_CHAR) };
            char::from_u32(mapped & 0xFFFF)
                .filter(|character| !character.is_control() && !character.is_whitespace())
        })
        .map_or_else(
            || label.to_string(),
            |character| character.to_uppercase().collect(),
        )
}

/// The punctuation key that types `character` on the active layout, for a
/// shortcut written with the character (`ctrl+;`).
pub(super) fn punctuation_key(character: char) -> Option<u32> {
    let mut units = [0u16; 2];
    let [unit] = character.encode_utf16(&mut units) else {
        return None;
    };
    // SAFETY: a pure lookup in this thread's keyboard layout.
    let scanned = unsafe { VkKeyScanW(*unit) };
    let vk = scanned as u16 & 0xFF;
    (scanned != -1
        && CAPTURE_NAMED_KEYS
            .iter()
            .any(|&(named, name)| named == vk && name.starts_with("oem ")))
    .then_some(u32::from(vk))
}

pub(super) const MOUSE_KEYS: [(u16, &str); 5] = [
    (0x01, "mouse1"),
    (0x02, "mouse2"),
    (0x04, "mouse3"),
    (0x05, "mouse4"),
    (0x06, "mouse5"),
];

pub(crate) fn virtual_key(key: &str) -> Result<u32> {
    let normalized = key.trim().to_ascii_lowercase();
    if let Some(number) = HotkeyKey::mouse_number(&normalized) {
        return MOUSE_KEYS
            .get(usize::from(number - 1))
            .map(|&(vk, _)| u32::from(vk))
            .ok_or_else(|| eyre!("Windows supports Mouse 1 through Mouse 5"));
    }
    if let Some((vk, _)) = CAPTURE_NAMED_KEYS
        .into_iter()
        .find(|&(_, name)| name == normalized)
    {
        return Ok(u32::from(vk));
    }
    let virtual_key = match normalized.as_str() {
        "return" => u32::from(VK_RETURN),
        "escape" => u32::from(VK_ESCAPE),
        "num enter" => NUMPAD_ENTER,
        name if HotkeyKey::special_name(name) && name != "menu" && name != "pause" => 0x91,
        "page_up" => 0x21,
        "page_down" => 0x22,
        _ => {
            if let Some(number) = normalized
                .strip_prefix('f')
                .and_then(|number| number.parse::<u32>().ok())
                .filter(|number| (1..=24).contains(number))
            {
                0x70 + number - 1
            } else if let Some(character) = single_character(&normalized) {
                if character.is_ascii_alphanumeric() {
                    u32::from(character.to_ascii_uppercase() as u8)
                } else if let Some(vk) = punctuation_key(character) {
                    vk
                } else {
                    return Err(eyre!(
                        "{key} isn't a key on the active Windows keyboard layout"
                    ));
                }
            } else {
                return Err(eyre!("unsupported Windows shortcut key: {key}"));
            }
        }
    };
    Ok(virtual_key)
}

pub(super) fn single_character(name: &str) -> Option<char> {
    let mut characters = name.chars();
    characters.next().filter(|_| characters.next().is_none())
}
