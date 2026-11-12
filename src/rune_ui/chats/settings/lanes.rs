//! Chats › Settings for the lanes: Stop all's shortcut, real input in the
//! window in front, the hidden browser for web tasks, the apps no task may
//! touch, how long a task may run and whether it waits out a usage limit,
//! and the agents' own Space (experimental).

use rune::{El, Tx, row, text};
use rune_kit::TextInput;

use super::super::super::Menu;
use super::super::super::controls::{
    CardRow, Paint, described_row, keycaps, note, pressable_row, row_switch, section,
    section_with, setting_row, toggle_row,
};
use super::super::super::general::quiet_select;
use super::super::super::list_detail as ld;
use super::super::super::lists::small_button;
use super::super::{Ui, setup};
use crate::desktop::i18n::{tr, tr_fill};

/// Task timeouts offered, in minutes; 0 never stops one.
const TIMEOUTS: [u32; 7] = [0, 5, 10, 15, 30, 60, 120];

static REMOVE_ICON: &rune::Icon = &rune_theme_opencode::icons::CLOSE_SMALL;

/// Stop all's shortcut, shown here and changed in General.
pub(in crate::rune_ui) fn stop_all_row(ui: &Ui, p: Paint) -> El {
    let caps = ui
        .settings
        .assistant
        .stop_all_hotkey
        .as_ref()
        .map(|binding| super::super::super::settings::shortcut_caps(binding, false));
    let keys = match caps {
        Some(caps) => keycaps(p, "keys", &caps),
        None => text(tr("Off"))
            .key("keys")
            .font(p.k.ty.small)
            .color(p.muted()),
    };
    setting_row(
        p,
        "assistant-stop-all",
        tr("Stop all tasks"),
        Some(tr("Stops every task at once, from any app")),
        row()
            .key("stop-all-controls")
            .gap(8.0)
            .center_y()
            .child(keys)
            .child(small_button(
                p,
                "change-stop-all",
                tr("Change in General…"),
                |ui: &mut Ui, tx: &mut Tx| {
                    super::flush(ui);
                    ui.select_pane(super::super::super::Pane::General, tx);
                },
            )),
    )
}

/// Whether a question about the window in front may use the real pointer
/// and keys there.
pub(in crate::rune_ui) fn here_input_row(ui: &Ui, p: Paint) -> CardRow {
    let on = ui.settings.assistant.here_physical_input;
    let description = if on {
        (
            tr("A question about the window in front may move your pointer and type there, under a “Hands off” pill. Keep your hands still until it's gone").to_owned(),
            p.muted(),
        )
    } else {
        (
            tr("Off keeps questions about the window in front in the background too, without your pointer or keys").to_owned(),
            p.muted(),
        )
    };
    pressable_row(
        described_row(
            p,
            "assistant-here-input",
            tr("Real input in the window in front"),
            Some(description),
            row_switch(p, on),
        ),
        on,
    )
    .on_press(|ui: &mut Ui| {
        ui.update(|settings| settings.assistant.here_physical_input ^= true);
    })
    .into()
}

/// The hidden browser for web tasks, and whether agent-browser is here.
pub(in crate::rune_ui) fn web_section(ui: &Ui, p: Paint) -> El {
    let on = ui.settings.assistant.background_browser;
    let mut rows: Vec<CardRow> = vec![
        toggle_row(
            p,
            "assistant-background-browser",
            tr("Hidden browser for web tasks"),
            tr("Each web task gets its own hidden browser (agent-browser), so many run at once and none uses your windows"),
            on,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.background_browser ^= true);
            },
        )
        .into(),
    ];
    let missing = ui
        .chats
        .settings
        .browser
        .as_ref()
        .is_some_and(|found| found.path.is_none() || !found.browser_ready);
    if on && missing {
        rows.push(
            described_row(
                p,
                "assistant-browser-missing",
                tr("agent-browser isn't installed"),
                Some((
                    tr("Until it is, web tasks use your own browser").to_owned(),
                    p.warning(),
                )),
                small_button(p, "set-up-browser", tr("Set up…"), |ui: &mut Ui, _| {
                    setup::open(ui);
                    setup::go(ui, setup::Page::Browser);
                }),
            )
            .into(),
        );
    }
    section(p, "assistant-web", tr("Web"), rows)
}

/// The apps no task may read or operate: each with Remove, a field to add
/// one by name, and the running apps to pick from.
pub(in crate::rune_ui) fn blocked_section(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let blocked = &ui.settings.assistant.blocked_apps;
    let mut rows: Vec<CardRow> = vec![
        note(
            "blocked-note",
            tr("No task reads, clicks, or types in these apps, whatever it's asked"),
            p.muted(),
        )
        .into(),
    ];
    for (index, app) in blocked.iter().enumerate() {
        let remove = app.clone();
        rows.push(
            setting_row(
                p,
                &format!("blocked{index}"),
                app,
                None,
                ld::ghost_icon(
                    p,
                    "remove",
                    REMOVE_ICON,
                    &tr_fill("Remove {}", app),
                    move |ui: &mut Ui, _| {
                        let remove = remove.clone();
                        ui.update(|settings| {
                            settings.assistant.blocked_apps.retain(|app| *app != remove)
                        });
                    },
                ),
            )
            .into(),
        );
    }
    rows.push(
        row()
            .key("add-blocked")
            .px(16.0)
            .py(10.0)
            .gap(8.0)
            .center_y()
            .child(
                TextInput::new(&ui.chats.settings.blocked)
                    .placeholder(tr("App name or bundle ID"))
                    .label(tr("Block an app"))
                    .width(None)
                    .on_edit(|ui: &mut Ui| &mut ui.chats.settings.blocked)
                    .on_submit(|ui: &mut Ui, _| add_typed(ui))
                    .build(p.k)
                    .key("blocked-field")
                    .grow(1.0)
                    .shrink(),
            )
            .child(small_button(p, "add-blocked-app", tr("Add"), |ui: &mut Ui, _| {
                add_typed(ui)
            }))
            .into(),
    );
    section_with(
        p,
        "assistant-blocked",
        tr("Blocked apps"),
        running_picker(ui, p, layer),
        rows,
    )
}

/// Adds the app typed in the field.
fn add_typed(ui: &mut Ui) {
    let app = ui.chats.settings.blocked.text.trim().to_owned();
    if block(ui, app) {
        ui.chats.settings.blocked.clear();
    }
}

/// Adds `app` to the blocked apps, once. True when it's there now.
fn block(ui: &mut Ui, app: String) -> bool {
    if app.is_empty() {
        return false;
    }
    let known = ui
        .settings
        .assistant
        .blocked_apps
        .iter()
        .any(|blocked| blocked.eq_ignore_ascii_case(&app));
    known || ui.update(|settings| settings.assistant.blocked_apps.push(app))
}

/// "Add a running app…": the apps running now that aren't blocked yet.
fn running_picker(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let blocked = &ui.settings.assistant.blocked_apps;
    let apps: Vec<String> = ui
        .chats
        .settings
        .running
        .iter()
        .flatten()
        .filter(|app| !blocked.iter().any(|known| known.eq_ignore_ascii_case(app)))
        .cloned()
        .collect();
    let mut labels = vec![tr("Add a running app…").to_owned()];
    labels.extend(apps.iter().cloned());
    quiet_select(
        ui,
        p,
        Menu::Pick("assistant-blocked-running"),
        "assistant-blocked-running-control".to_owned(),
        tr("Add a running app…"),
        labels,
        0,
        move |ui, chosen| {
            if let Some(app) = chosen.checked_sub(1).and_then(|index| apps.get(index)) {
                block(ui, app.clone());
            }
        },
        layer,
    )
}

/// How long a task may run, and whether it waits out a usage limit.
pub(in crate::rune_ui) fn task_rows(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> Vec<El> {
    let saved = ui.settings.assistant.task_timeout_minutes;
    let (values, selected) = super::with_saved(&TIMEOUTS, saved);
    let timeout = quiet_select(
        ui,
        p,
        Menu::Pick("assistant-timeout"),
        "assistant-timeout-control".to_owned(),
        tr("Stop a task after"),
        values
            .iter()
            .map(|minutes| match minutes {
                0 => tr("Never").to_owned(),
                _ => tr_fill("{} min", &minutes.to_string()),
            })
            .collect(),
        selected,
        move |ui, chosen| {
            let minutes = values[chosen];
            ui.update(|settings| settings.assistant.task_timeout_minutes = minutes);
        },
        layer,
    );
    vec![
        setting_row(
            p,
            "assistant-timeout",
            tr("Stop a task after"),
            Some(tr("A task that runs this long stops, with what it did so far")),
            timeout,
        ),
        toggle_row(
            p,
            "assistant-retry",
            tr("Wait out usage limits"),
            tr("A task that hits its agent's usage limit waits, then tries again"),
            ui.settings.assistant.retry_on_limit,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.retry_on_limit ^= true);
            },
        ),
    ]
}

/// The agents' own Space: off and disabled, with why, until this Mac can.
pub(in crate::rune_ui) fn experimental_section(ui: &Ui, p: Paint) -> El {
    let available = ui.chats.settings.space_available;
    let on = ui.settings.assistant.agent_space && available;
    let title = tr("A Space for the agents");
    let row = if available {
        toggle_row(
            p,
            "assistant-agent-space",
            title,
            tr("Windows background tasks open move to a Space of their own, out of your way"),
            on,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.agent_space ^= true);
            },
        )
    } else {
        described_row(
            p,
            "assistant-agent-space",
            title,
            Some((
                tr("macOS doesn't let Hex move windows between Spaces on this Mac yet").to_owned(),
                p.muted(),
            )),
            rune_kit::Switch::new(false)
                .disabled(true)
                .build(p.k)
                .key("switch")
                .a11y_hidden(),
        )
        .role("switch")
        .checked(false)
    };
    section(p, "assistant-experimental", tr("Experimental"), vec![row])
}
