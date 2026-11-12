//! The setup sheet as it draws: over the whole window in the first run's
//! type, one card per step with its live status, then Later and Finish.

use rune::{Align, El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant, RadioItem, Spinner, TextInput};

use super::super::controls::{
    ButtonKind, DESCRIPTION, METRICS, Paint, PushSize, button, keycaps, raised,
};
use super::super::lists::{Tone, permission_control, small_button, tone_tag};
use super::setup::{
    self, Report, SELF_TESTS, SetupSheet, State, Status, chosen, report,
};
use super::{BACKENDS, Ui, backend_name};
use crate::app_settings::AssistantBackend;
use crate::desktop::i18n::{tr, tr_fill};

static PASSED: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_CHECK;
static FAILED: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_X;
static CLOSE: &rune::Icon = &rune_theme_opencode::icons::CLOSE;

/// The steps' column.
const COLUMN: f32 = 600.0;
/// Where a step's rows start: past its status icon.
const ROW_INDENT: f32 = 32.0;

fn state_word(state: State) -> &'static str {
    match state {
        State::Pending => tr("Not checked yet"),
        State::Checking => tr("Checking…"),
        State::Passed => tr("Works"),
        State::Failed => tr("Needs attention"),
        State::Skipped => tr("Not needed"),
    }
}

/// A check's mark: an empty ring, a spinner, a green check, a red cross,
/// or a dash.
fn mark(p: Paint, state: State, size: f32) -> El {
    let c = &p.k.c;
    let mark = match state {
        State::Pending => el()
            .size(size - 4.0, size - 4.0)
            .radius((size - 4.0) / 2.0)
            .border(1.5, c.border_strong),
        State::Checking => Spinner::new().size(size - 2.0).color(p.accent()).build(p.k),
        State::Passed => icon(PASSED, size).color(p.success()),
        State::Failed => icon(FAILED, size).color(p.danger()),
        State::Skipped => el()
            .size(size - 8.0, 1.5)
            .radius(1.0)
            .fill(c.border_strong),
    };
    el().key("mark")
        .size(size, size)
        .center()
        .child(mark)
        .a11y_hidden()
}

fn detail_ink(p: Paint, state: State) -> rune::Color {
    match state {
        State::Failed => p.danger(),
        _ => p.muted(),
    }
}

/// One step: its mark, title, what it saw, an action, and its rows.
fn step(
    p: Paint,
    key: &str,
    title: &str,
    status: &Status,
    action: Option<El>,
    rows: Vec<El>,
) -> El {
    let detail = (!status.detail.is_empty()).then(|| {
        text(status.detail.clone())
            .key("detail")
            .font(DESCRIPTION)
            .color(detail_ink(p, status.state))
            .wrap()
            .a11y_hidden()
    });
    let said = if status.detail.is_empty() {
        state_word(status.state).to_owned()
    } else {
        format!("{}. {}", state_word(status.state), status.detail)
    };
    raised(p, key)
        .role("group")
        .label(title.to_owned())
        .description(said)
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .child(
            row()
                .key("head")
                .gap(12.0)
                .items_start()
                .child(mark(p, status.state, 20.0))
                .child(
                    col()
                        .key("copy")
                        .grow(1.0)
                        .shrink()
                        .gap(2.0)
                        .pt(1.0)
                        .child(
                            text(title.to_owned())
                                .key("title")
                                .font(p.k.ty.ui_medium)
                                .color(p.text())
                                .a11y_hidden(),
                        )
                        .children(detail),
                )
                .children(action),
        )
        .children(rows.into_iter().map(|row| row.ml(ROW_INDENT)))
}

/// A row inside a step: its own mark, name, what it saw, and a control.
fn check_row(p: Paint, key: &str, title: &str, status: &Status, control: Option<El>) -> El {
    let said = if status.detail.is_empty() {
        state_word(status.state).to_owned()
    } else {
        format!("{}. {}", state_word(status.state), status.detail)
    };
    row()
        .key(key.to_owned())
        .role("group")
        .label(title.to_owned())
        .description(said)
        .gap(10.0)
        .min_h(30.0)
        .center_y()
        .child(mark(p, status.state, 14.0))
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .shrink()
                .gap(1.0)
                .child(
                    text(title.to_owned())
                        .key("title")
                        .font(p.k.ty.ui)
                        .color(p.text())
                        .a11y_hidden(),
                )
                .children((!status.detail.is_empty()).then(|| {
                    text(status.detail.clone())
                        .key("detail")
                        .font(p.k.ty.small)
                        .color(detail_ink(p, status.state))
                        .wrap()
                        .a11y_hidden()
                })),
        )
        .children(control)
}

/// The sheet over the whole window, while it shows.
pub(in crate::rune_ui) fn overlay(ui: &Ui, p: Paint) -> Option<El> {
    let sheet = ui.chats.setup.as_ref()?;
    let report = report(ui, sheet);
    let lead = tr(
        "Hex checks each piece on this Mac as you go. Once every step is green, the Action shortcut, Agent mode, and Chats work.",
    );
    let heading = col()
        .key("heading")
        .gap(8.0)
        .items_center()
        .pb(8.0)
        .child(
            text(tr("Set up the assistant"))
                .key("title")
                .role("heading")
                .level(1)
                .font(super::super::onboarding_kit::title_font(p))
                .color(p.text()),
        )
        .child(
            text(lead)
                .key("lead")
                .font(super::super::onboarding_kit::LEAD)
                .color(p.muted())
                .align(Align::Center)
                .wrap()
                .max_w(480.0),
        );
    let steps = col()
        .key("steps")
        .w(COLUMN)
        .gap(12.0)
        .child(heading)
        .child(agents_step(ui, p, &report))
        .child(desktop_step(ui, sheet, p, &report))
        .child(permissions_step(ui, p, &report))
        .child(checks_step(sheet, p, &report))
        .child(shortcut_step(ui, p, &report));
    let content = col().key("setup-content").items_center().px(24.0).child(steps);
    let c = &p.k.c;
    let top = super::super::sidebar::title_bar(row())
        .key("setup-top")
        .h(52.0)
        .px(12.0)
        .center_y()
        .child(el().key("spacer").grow(1.0))
        .child(
            IconButton::new(CLOSE)
                .variant(IconButtonVariant::Ghost)
                .label(tr("Close"))
                .on_press(setup::close)
                .build(p.k)
                .key("close-setup"),
        );
    let ready = report.ready();
    let finish = Button::new(tr("Finish"))
        .variant(ButtonVariant::Contrast)
        .disabled(!ready)
        .on_press_tx(|ui: &mut Ui, tx: &mut Tx| setup::finish(ui, tx))
        .build_push(p.k)
        .key("finish-setup");
    let finish = if ready { finish.default_button() } else { finish };
    let footer = row()
        .key("setup-footer")
        .h(64.0)
        .px(METRICS.gutter)
        .gap(8.0)
        .center_y()
        .border_sides([p.hair, 0.0, 0.0, 0.0], c.border_weak)
        .child(
            text(tr_fill("{} of 5 steps work", &report.done().to_string()))
                .key("progress")
                .role("status")
                .font(DESCRIPTION)
                .color(p.muted()),
        )
        .child(el().key("spacer").grow(1.0))
        .child(button(
            p,
            "later-setup",
            tr("Later"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| setup::close(ui),
        ))
        .child(finish);
    Some(
        col()
            .key("assistant-setup")
            .role("dialog")
            .label(tr("Set up the assistant"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(c.bg_deep)
            .on_press(|_: &mut Ui| {})
            .child(top)
            .child(super::super::faded_body(p.k, "setup-scroll", content, (8.0, 24.0)))
            .child(footer)
            .modal(true, setup::close),
    )
}

/// The agents found, one chosen to answer, and the Claude API key.
fn agents_step(ui: &Ui, p: Paint, report: &Report) -> El {
    let found = ui.chats.settings.found.as_deref().unwrap_or_default();
    let chosen_backend = ui.settings.assistant.backend;
    let mut rows: Vec<El> = Vec::new();
    for (index, backend) in BACKENDS.iter().enumerate() {
        let Some(entry) = found.iter().find(|entry| entry.backend == *backend) else {
            continue;
        };
        let description = match (&entry.version, &entry.note) {
            (_, Some(note)) if !entry.ready => note.clone(),
            (Some(version), _) => tr_fill("Version {}", version),
            (None, _) => String::new(),
        };
        let (tag, tone) = if entry.ready {
            (tr("Ready"), Tone::Success)
        } else if entry.path.is_some() || *backend == AssistantBackend::ClaudeApi {
            (tr("Not ready"), Tone::Warning)
        } else {
            (tr("Not found"), Tone::Neutral)
        };
        let pick = *backend;
        let radio = RadioItem::new(backend_name(*backend), *backend == chosen_backend)
            .description(description)
            .on_select(move |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.backend = pick);
            })
            .build(p.k)
            .key("radio")
            .grow(1.0)
            .shrink();
        rows.push(
            row()
                .key(format!("agent{index}"))
                .gap(12.0)
                .min_h(34.0)
                .center_y()
                .child(radio)
                .child(tone_tag(p, "tag", tag, tone)),
        );
    }
    if chosen_backend == AssistantBackend::ClaudeApi && !chosen(ui).is_some_and(|found| found.ready) {
        rows.push(key_row(ui, p));
    }
    step(
        p,
        "setup-agents",
        tr("An agent to answer"),
        &report.agents,
        Some(small_button(p, "detect-again", tr("Check again"), |ui: &mut Ui, _| {
            setup::detect(ui)
        })),
        rows,
    )
}

/// The Claude API key, saved to the Keychain.
fn key_row(ui: &Ui, p: Paint) -> El {
    let state = &ui.chats.settings;
    row()
        .key("api-key")
        .gap(8.0)
        .center_y()
        .child(
            TextInput::new(&state.key)
                .secure()
                .placeholder(tr("Claude API key"))
                .label(tr("Claude API key"))
                .width(None)
                .on_edit(|ui: &mut Ui| &mut ui.chats.settings.key)
                .on_submit(|ui: &mut Ui, _| super::settings::save_key(ui))
                .invalid(state.key_error.is_some())
                .build(p.k)
                .key("api-key-field")
                .grow(1.0)
                .shrink(),
        )
        .child(small_button(p, "save-api-key", tr("Save"), |ui: &mut Ui, _| {
            super::settings::save_key(ui)
        }))
}

/// agent-desktop: found, or Install.
fn desktop_step(ui: &Ui, sheet: &SetupSheet, p: Paint, report: &Report) -> El {
    let missing = sheet.desktop.as_ref().is_some_and(|found| found.path.is_none())
        || sheet.install_error.is_some();
    let action = match report.desktop.state {
        State::Skipped | State::Checking => None,
        _ if missing => Some(button(
            p,
            "install-agent-desktop",
            tr("Install"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| setup::install(ui),
        )),
        _ => None,
    };
    let mut rows = Vec::new();
    if missing && ui.settings.assistant.control_desktop {
        rows.push(
            text(tr("Installs with Homebrew: brew install agent-desktop"))
                .key("install-hint")
                .font(p.k.ty.small)
                .color(p.muted())
                .wrap(),
        );
    }
    step(
        p,
        "setup-desktop",
        "agent-desktop",
        &report.desktop,
        action,
        rows,
    )
}

/// Hex's permissions and agent-desktop's, each with Allow.
fn permissions_step(ui: &Ui, p: Paint, report: &Report) -> El {
    let rows = report
        .grants
        .iter()
        .map(|(grant, status)| {
            let grant = *grant;
            let control = (status.state == State::Failed).then(|| {
                let state = match grant.kind() {
                    Some(kind) => ui.platform.control_for(kind, false),
                    None => super::super::lists::PermissionControl::Allow,
                };
                permission_control(
                    p,
                    &format!("{}-allow", grant.key()),
                    state,
                    ButtonKind::Secondary,
                    move |ui: &mut Ui| setup::allow(ui, grant),
                )
            });
            check_row(
                p,
                grant.key(),
                &format!("{} · {}", grant.name(), grant.owner()),
                status,
                control,
            )
        })
        .collect();
    step(
        p,
        "setup-permissions",
        tr("Permissions"),
        &report.permissions(),
        None,
        rows,
    )
}

/// The self-tests, run on demand.
fn checks_step(sheet: &SetupSheet, p: Paint, report: &Report) -> El {
    let running = sheet.tests.iter().any(|test| test.state == State::Checking);
    let can_run = report.agents.state == State::Passed && !running;
    let label = if sheet.tests_ran {
        tr("Run again")
    } else {
        tr("Run checks")
    };
    let run = Button::new(label)
        .variant(ButtonVariant::Neutral)
        .disabled(!can_run)
        .on_press(|ui: &mut Ui| setup::run_tests(ui))
        .build_push(p.k)
        .key("run-checks");
    let rows = SELF_TESTS
        .iter()
        .zip(&report.tests)
        .map(|(check, status)| check_row(p, check.id, tr(check.title), status, None))
        .collect();
    let mut status = report.checks();
    if status.state == State::Pending && report.agents.state != State::Passed {
        status.detail = tr("Choose an agent that's ready first").into();
    } else if status.state == State::Pending {
        status.detail = tr("Sends a real request through each piece").into();
    }
    step(p, "setup-checks", tr("Checks"), &status, Some(run), rows)
}

/// One spoken question through the Action shortcut.
fn shortcut_step(ui: &Ui, p: Paint, report: &Report) -> El {
    let mut rows = Vec::new();
    if let Some(binding) = ui.settings.assistant.action_hotkey.as_ref() {
        let caps = super::super::settings::shortcut_caps(binding, false);
        rows.push(
            row()
                .key("try-it")
                .gap(10.0)
                .center_y()
                .flex_wrap()
                .child(
                    text(tr("Hold"))
                        .key("hold")
                        .font(p.k.ty.ui)
                        .color(p.text()),
                )
                .child(keycaps(p, "action-keys", &caps))
                .child(
                    text(tr("and ask something, like “What time is it in Tokyo?”"))
                        .key("ask")
                        .font(p.k.ty.ui)
                        .color(p.text())
                        .wrap()
                        .shrink(),
                ),
        );
    }
    step(
        p,
        "setup-shortcut",
        tr("Try the Action shortcut"),
        &report.shortcut,
        None,
        rows,
    )
}
