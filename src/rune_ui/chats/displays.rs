//! "Agents work on": which display Background tasks open their windows on.
//! Auto picks the display you're not working on. Settings shows it with a
//! small map of the displays; the onboarding offers it as one optional row
//! when two or more are connected.

use rune::{El, el, row};

use super::super::Menu;
use super::super::controls::{Paint, setting_row};
use super::super::general::quiet_select;
use super::Ui;
use crate::assistant::computer::{self, Display};
use crate::desktop::i18n::tr;

/// The map's size, in points.
const MAP_W: f32 = 84.0;
const MAP_H: f32 = 36.0;

/// Reads the connected displays (live only: scenes keep theirs).
pub(super) fn read(ui: &mut Ui) {
    if ui.chats.live() {
        ui.chats.settings.displays = computer::displays();
    }
}

/// The display agents use now: the chosen one, or Auto's pick (the first
/// you're not working on).
fn used<'a>(displays: &'a [Display], chosen: Option<&str>) -> Option<&'a Display> {
    computer::agents_display(displays, chosen)
}

fn choose(ui: &mut Ui, display: Option<String>) {
    ui.update(|settings| settings.assistant.agent_display = display);
}

/// A small map of the displays as they're arranged, the agents' one
/// filled.
fn map(p: Paint, displays: &[Display], chosen: Option<&str>) -> El {
    let k = p.k;
    let used = used(displays, chosen).map(|display| display.id.clone());
    let left = displays
        .iter()
        .map(|d| d.frame.0)
        .fold(f64::INFINITY, f64::min);
    let top = displays
        .iter()
        .map(|d| d.frame.1)
        .fold(f64::INFINITY, f64::min);
    let right = displays
        .iter()
        .map(|d| d.frame.0 + d.frame.2)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = displays
        .iter()
        .map(|d| d.frame.1 + d.frame.3)
        .fold(f64::NEG_INFINITY, f64::max);
    let scale = (f64::from(MAP_W) / (right - left).max(1.0))
        .min(f64::from(MAP_H) / (bottom - top).max(1.0));
    let mut map = el()
        .key("display-map")
        .role("img")
        .label(tr("Display arrangement"))
        .size(MAP_W, MAP_H);
    for (index, display) in displays.iter().enumerate() {
        let (x, y, w, h) = display.frame;
        let agents = used.as_deref() == Some(display.id.as_str());
        let mut screen = el()
            .key(format!("screen{index}"))
            .absolute(((x - left) * scale) as f32, ((y - top) * scale) as f32)
            .size(
                ((w * scale) as f32 - 2.0).max(4.0),
                ((h * scale) as f32 - 2.0).max(4.0),
            )
            .radius(2.0)
            .center();
        screen = if agents {
            screen.fill(p.accent()).border(1.0, p.accent())
        } else {
            screen.fill(k.c.layer_02).border(1.0, k.c.border_strong)
        };
        if display.has_focus {
            // You: a dot on the display you're working on.
            screen = screen.child(el().key("you").size(4.0, 4.0).radius(2.0).fill(if agents {
                k.c.bg_base
            } else {
                p.muted()
            }));
        }
        map = map.child(screen.a11y_hidden());
    }
    map
}

/// Auto, then each display by name.
fn labels(displays: &[Display]) -> Vec<String> {
    let mut labels = vec![tr("Auto: the display you're not on").to_owned()];
    labels.extend(displays.iter().map(|display| display.name.clone()));
    labels
}

fn selected(displays: &[Display], chosen: Option<&str>) -> usize {
    chosen
        .and_then(|id| displays.iter().position(|display| display.id == id))
        .map_or(0, |index| index + 1)
}

/// Settings' row: the picker and the map.
pub(super) fn settings_row(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let displays = ui.chats.settings.displays.clone();
    let chosen = ui.settings.assistant.agent_display.clone();
    let picker = {
        let ids: Vec<String> = displays.iter().map(|display| display.id.clone()).collect();
        quiet_select(
            ui,
            p,
            Menu::Pick("assistant-agent-display"),
            "assistant-agent-display-control".to_owned(),
            tr("Agents work on"),
            labels(&displays),
            selected(&displays, chosen.as_deref()),
            move |ui, chosen| {
                choose(
                    ui,
                    chosen
                        .checked_sub(1)
                        .and_then(|index| ids.get(index).cloned()),
                )
            },
            layer,
        )
    };
    let control = row()
        .key("agent-display-controls")
        .gap(12.0)
        .center_y()
        .children((displays.len() > 1).then(|| map(p, &displays, chosen.as_deref())))
        .child(picker);
    setting_row(
        p,
        "assistant-agent-display",
        tr("Agents work on"),
        Some(tr("Windows background tasks open go to this display")),
        control,
    )
}

/// The onboarding's optional row, with two or more displays.
pub(super) fn onboarding_row(ui: &Ui, p: Paint) -> Option<El> {
    let displays = &ui.chats.settings.displays;
    if displays.len() < 2 {
        return None;
    }
    let chosen = ui.settings.assistant.agent_display.clone();
    let picked = selected(displays, chosen.as_deref());
    let mut chips = row()
        .key("display-chips")
        .role("radiogroup")
        .gap(6.0)
        .flex_wrap();
    for (index, label) in labels(displays).into_iter().enumerate() {
        let id = index
            .checked_sub(1)
            .and_then(|index| displays.get(index))
            .map(|display| display.id.clone());
        let label = if index == 0 {
            tr("Auto").to_owned()
        } else {
            label
        };
        chips = chips.child(super::agent_picker::choice(
            p,
            format!("display{index}"),
            &label,
            index == picked,
            move |ui| choose(ui, id.clone()),
        ));
    }
    Some(
        row()
            .key("agent-display")
            .role("group")
            .label(tr("Agents work on"))
            .min_h(56.0)
            .py(10.0)
            .px(16.0)
            .gap(12.0)
            .center_y()
            .child(map(p, displays, chosen.as_deref()))
            .child(
                rune::col()
                    .key("copy")
                    .grow(1.0)
                    .w(0.0)
                    .gap(6.0)
                    .child(
                        rune::text(tr("Agents work on (optional)"))
                            .key("title")
                            .font(p.k.ty.ui_medium)
                            .color(p.text()),
                    )
                    .child(chips),
            ),
    )
}

/// Two displays side by side, the pointer on the main one, for scenes.
pub(in crate::rune_ui) fn fixture() -> Vec<Display> {
    vec![
        Display {
            id: "1".into(),
            name: "Built-in Retina Display".into(),
            frame: (0.0, 0.0, 1512.0, 982.0),
            visible_frame: (0.0, 25.0, 1512.0, 957.0),
            scale: 2.0,
            is_primary: true,
            has_focus: true,
        },
        Display {
            id: "2".into(),
            name: "Studio Display".into(),
            frame: (1512.0, -300.0, 2560.0, 1440.0),
            visible_frame: (1512.0, -300.0, 2560.0, 1440.0),
            scale: 2.0,
            is_primary: false,
            has_focus: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_uses_the_display_you_are_not_on() {
        let displays = fixture();
        assert_eq!(used(&displays, None).map(|d| d.id.as_str()), Some("2"));
        assert_eq!(used(&displays, Some("1")).map(|d| d.id.as_str()), Some("1"));
        assert_eq!(selected(&displays, Some("2")), 2);
        assert_eq!(selected(&displays, None), 0);
    }
}
