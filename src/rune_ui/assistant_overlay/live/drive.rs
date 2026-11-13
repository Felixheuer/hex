//! The person driving a browser from its large view: their keys and
//! pointer as the page's input.

use super::*;

/// The page's key for a key the person pressed on the large view, while
/// they drive it; `None` for what the window keeps (⌘ shortcuts, Escape).
pub(crate) fn key_input(key: &rune::Key, mods: rune::Mods) -> Option<Input> {
    use rune::Key;
    if mods.cmd {
        return None;
    }
    let named = |name: &str, text: Option<&str>| {
        Some(Input::Key {
            key: name.to_owned(),
            text: text.map(str::to_owned),
        })
    };
    match key {
        Key::Text(text) if !text.is_empty() => named(text, Some(text)),
        Key::Enter => named("Enter", Some("\r")),
        Key::Backspace => named("Backspace", None),
        Key::Delete => named("Delete", None),
        Key::Tab => named("Tab", None),
        Key::Left => named("ArrowLeft", None),
        Key::Right => named("ArrowRight", None),
        Key::Up => named("ArrowUp", None),
        Key::Down => named("ArrowDown", None),
        Key::Home => named("Home", None),
        Key::End => named("End", None),
        Key::PageUp => named("PageUp", None),
        Key::PageDown => named("PageDown", None),
        _ => None,
    }
}

/// Where a point on a preview drawn `drawn` big lands on the page, in its
/// CSS pixels.
pub(crate) fn to_page(point: (f32, f32), drawn: (f32, f32), viewport: (f32, f32)) -> (f32, f32) {
    if drawn.0 <= 0.0 || drawn.1 <= 0.0 {
        return (0.0, 0.0);
    }
    (
        (point.0 / drawn.0 * viewport.0).clamp(0.0, viewport.0),
        (point.1 / drawn.1 * viewport.1).clamp(0.0, viewport.1),
    )
}
