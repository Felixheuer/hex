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
    if on && !missing && ui.chats.settings.browser.is_some() {
        rows.push(engine_row(ui, p));
        rows.push(sign_in_row(ui, p));
    }
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

/// Agent windows parked off-screen (experimental): macOS has no way for
/// Hex to give them a Space of their own.
pub(in crate::rune_ui) fn experimental_section(ui: &Ui, p: Paint) -> El {
    let row = toggle_row(
        p,
        "assistant-agent-space",
        tr("Park agent windows off-screen (experimental)"),
        tr("Windows background tasks open move out of sight until you bring one forward from the task list. A Space of their own isn't possible on current macOS"),
        ui.settings.assistant.agent_space,
        |ui: &mut Ui| {
            ui.update(|settings| settings.assistant.agent_space ^= true);
        },
    );
    section(p, "assistant-experimental", tr("Experimental"), vec![row])
}

/// Signing in to sites in Hex's own browser, which every task's hidden
/// browser then starts from.
fn sign_in_row(ui: &Ui, p: Paint) -> CardRow {
    let state = &ui.chats.settings;
    let (detail, ink) = match &state.sign_in_error {
        Some(error) => (error.clone(), p.danger()),
        None if state.signing_in => (
            tr("Sign in in Hex's browser window, then press Done").to_owned(),
            p.accent(),
        ),
        None if state.signed_in => (
            tr("Tasks start signed in where you signed in").to_owned(),
            p.muted(),
        ),
        None => (
            tr("Hidden browsers start signed out. Sign in once in Hex's own browser").to_owned(),
            p.muted(),
        ),
    };
    let mut controls = row()
        .key("sign-in-controls")
        .gap(8.0)
        .center_y()
        .child(super::super::sheet::sign_in_button(ui, p, "sign-in-sites"));
    if state.signed_in && !state.signing_in {
        controls = controls.child(small_button(
            p,
            "forget-sign-ins",
            tr("Sign out everywhere"),
            |ui: &mut Ui, _| ui.chats.jobs.run(super::super::Job::ForgetSignIn),
        ));
    }
    described_row(
        p,
        "assistant-sign-in",
        tr("Sites tasks may use"),
        Some((detail, ink)),
        controls,
    )
    .into()
}

/// Which engine clean browsers run: Lightpanda (light, for many at once)
/// or Chrome, with Lightpanda one click away.
fn engine_row(ui: &Ui, p: Paint) -> CardRow {
    use crate::assistant::browser::Engine;
    let state = &ui.chats.settings;
    let (detail, control) = match state.engine {
        Some(Engine::Lightpanda) => (
            tr("Lightpanda: about ten times lighter, so many research tasks run at once").to_owned(),
            text(tr("Lightpanda"))
                .key("engine")
                .font(p.k.ty.small_medium)
                .color(p.muted()),
        ),
        _ if state.installing_lightpanda => (
            tr("Installing Lightpanda…").to_owned(),
            rune_kit::Spinner::new()
                .size(14.0)
                .color(p.muted())
                .build(p.k)
                .key("installing"),
        ),
        _ => (
            tr("Chrome. Lightpanda is about ten times lighter for parallel research").to_owned(),
            small_button(p, "install-lightpanda", tr("Install Lightpanda"), |ui: &mut Ui, _| {
                ui.chats.settings.installing_lightpanda = true;
                ui.chats
                    .jobs
                    .run(super::super::Job::Fix(crate::assistant::backends::Fix::InstallLightpanda));
            }),
        ),
    };
    described_row(
        p,
        "assistant-browser-engine",
        tr("Clean browsers run on"),
        Some((detail, p.muted())),
        control,
    )
    .into()
}
